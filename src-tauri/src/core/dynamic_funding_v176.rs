//! Funding is a financial authorization, not an inference output-token limit.
//! All network calls run outside the local model's token production loop.
use crate::core::{auth_client::SupabaseAuthClient, production_task_http::{self, LocalAuth}, task_client::TaskEnvelope};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};

const PROTOCOL: &str = "dynamic-funding-v176";
const CONTROL: &str = "/v1/chat/completions/control";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Usage { raw: u64, billing: u64 }
impl Usage {
    fn tokens(self) -> u64 { self.billing / 4 + u64::from(self.billing % 4 != 0) }
    fn final_output(output: &str) -> Self {
        Self { raw: output.len() as u64, billing: output.encode_utf16().count() as u64 }
    }
}

#[derive(Clone, Debug)]
struct Frame { sequence: u64, usage: Usage, elapsed: u64 }
impl Frame {
    fn event(&self) -> &'static str {
        if self.sequence == 1 { "generation.started" } else { "generation.progress" }
    }
    fn body(&self, task_id: &Value, email: &str, hardware: &str, execution: &str) -> Value {
        json!({"taskId":task_id,"providerEmail":email,"hardwareId":hardware,
            "executionId":execution,"event":self.event(),"sequence":self.sequence,
            "outputChars":self.usage.raw,"outputBillingCharsV176":self.usage.billing,
            "elapsedMs":self.elapsed})
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinancialState { Funded, Limited, Unconfirmed }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Receipt { grant: Option<u64>, state: FinancialState, reload: bool }
impl Receipt {
    fn parse(v: &Value, frame: &Frame, execution: &str) -> Result<Self, String> {
        if v["ok"].as_bool() != Some(true) || v["event"].as_str() != Some(frame.event()) ||
           v["sequence"].as_u64() != Some(frame.sequence) || v["executionId"].as_str() != Some(execution) {
            return Err("funding_progress_ack_identity_mismatch".into());
        }
        let r = &v["dynamicFundingV176"];
        if r["protocol"].as_str() != Some(PROTOCOL) || r["sequence"].as_u64() != Some(frame.sequence) ||
            r["executionId"].as_str() != Some(execution) {
            return Err("funding_receipt_identity_mismatch".into());
        }
        let state = match r["status"].as_str() {
            Some("funded") => FinancialState::Funded,
            Some("funding_limited") => FinancialState::Limited,
            Some("funding_unconfirmed") => FinancialState::Unconfirmed,
            _ => return Err("funding_receipt_status_invalid".into())
        };
        let grant = if state == FinancialState::Unconfirmed { None } else {
            Some(r["authorizedOutputBillingTokens"].as_u64()
                .filter(|n| *n <= u64::MAX/4).ok_or("funding_grant_invalid")?)
        };
        Ok(Self { grant, state, reload: state == FinancialState::Limited && r["reloadRequired"].as_bool() == Some(true) })
    }
}

#[derive(Clone, Debug)]
struct RuntimeUsage { usage: Usage, paused: bool, grant: u64 }

fn loopback(base: &str) -> bool {
    reqwest::Url::parse(base).ok().map(|u| u.scheme()=="http" &&
        u.host_str()==Some("127.0.0.1") && u.port().is_some() && u.username().is_empty() &&
        u.password().is_none() && u.path()=="/" && u.query().is_none() && u.fragment().is_none()).unwrap_or(false)
}

fn local_client() -> Result<Client,String> {
    Client::builder().no_proxy().timeout(Duration::from_secs(3)).build()
        .map_err(|_| "funding_local_client_failed".into())
}
fn local_call(http: &Client, base: &str, body: &Value) -> Result<Value,String> {
    http.post(format!("{base}{CONTROL}")).json(body).send()
        .and_then(|r| r.error_for_status()).and_then(|r| r.json::<Value>())
        .map_err(|_| "funding_runtime_control_unconfirmed".into())
}
fn probe(http: &Client, base: &str) -> Result<(),String> {
    if !loopback(base) { return Err("funding_nonlocal_runtime_rejected".into()); }
    let v=local_call(http,base,&json!({"action":"swarm_funding_probe_v176"}))?;
    if v["success"].as_bool()!=Some(true) || v["protocol"].as_str()!=Some(PROTOCOL) {
        return Err("funding_runtime_not_compatible".into());
    }
    Ok(())
}
fn runtime_usage(http: &Client, base: &str, id: &str, grant: u64) -> Result<Option<RuntimeUsage>,String> {
    let v=local_call(http,base,&json!({"action":"swarm_funding_v176","id":id,"authorizedOutputBillingTokens":grant}))?;
    if v["success"].as_bool()==Some(false) && v["message"].as_str()==Some("no active funded completion") { return Ok(None); }
    let r=&v["swarmFundingV176"];
    if v["success"].as_bool()!=Some(true) || r["protocol"].as_str()!=Some(PROTOCOL) { return Err("funding_runtime_reply_invalid".into()); }
    let raw=r["outputChars"].as_u64().ok_or("funding_runtime_raw_invalid")?;
    let billing=r["outputBillingChars"].as_u64().ok_or("funding_runtime_billing_invalid")?;
    if billing>raw.saturating_mul(6).saturating_add(15) { return Err("funding_runtime_meter_invalid".into()); }
    let local_grant=r["authorizedOutputBillingTokens"].as_u64().ok_or("funding_runtime_grant_invalid")?;
    if local_grant!=grant { return Err("funding_runtime_grant_mismatch".into()); }
    Ok(Some(RuntimeUsage{usage:Usage{raw,billing},paused:r["fundingPaused"].as_bool().ok_or("funding_runtime_pause_invalid")?,grant:local_grant}))
}

#[derive(Debug)]
enum SendError { Retry, Fenced }

fn send_frame(http: &Client, auth_client: &SupabaseAuthClient, auth: &mut LocalAuth, body: &Value) -> Result<Value,SendError> {
    for attempt in 0..2 {
        let header=production_task_http::bearer(&auth.access_token).map_err(|_|SendError::Fenced)?;
        let response=http.post(format!("{}/enterprise/task-progress",production_task_http::api_base()))
            .header(reqwest::header::AUTHORIZATION,header).json(body).send().map_err(|_|SendError::Retry)?;
        let status=response.status().as_u16();
        if status==401 && attempt==0 {
            let old=auth.access_token.clone();
            *auth=production_task_http::refresh_auth(auth_client,&old).map_err(|_|SendError::Retry)?;
            continue;
        }
        if matches!(status,200|201|202) { return response.json::<Value>().map_err(|_|SendError::Retry); }
        if matches!(status,408|425|429) || status>=500 { return Err(SendError::Retry); }
        return Err(SendError::Fenced);
    }
    Err(SendError::Retry)
}

#[derive(Debug)]
struct Shared {
    grant: u64, financial: FinancialState, paused: bool, reload: bool,
    fatal: Option<String>, final_usage: Option<Usage>, observed_raw: u64,
    saw_pause: bool, saw_reload: bool, saw_unconfirmed: bool,
}
impl Shared {
    fn new(grant: u64) -> Self { Self {grant,financial:FinancialState::Funded,paused:false,reload:false,
        fatal:None,final_usage:None,observed_raw:0,saw_pause:false,saw_reload:false,saw_unconfirmed:false} }
    fn receipt(&mut self, r: Receipt) {
        if r.grant.map(|n| n < self.grant).unwrap_or(false) { return; }
        if let Some(grant)=r.grant { self.grant=self.grant.max(grant); }
        self.financial=r.state;
        self.reload=r.reload;
        self.saw_unconfirmed |= r.state==FinancialState::Unconfirmed;
        self.saw_reload |= r.reload;
    }
    fn unconfirmed(&mut self) { self.financial=FinancialState::Unconfirmed; self.reload=false; self.saw_unconfirmed=true; }
}

pub(crate) struct FundingSessionV176 {
    id: String, initial_grant: u64, shared: Arc<Mutex<Shared>>,
    done: Arc<AtomicBool>, node_stop: Arc<AtomicBool>, worker: Option<thread::JoinHandle<()>>,
    deadline: Instant,
}

fn retrieval_delivery_compatible_v176(
    stage: Option<&str>, model: Option<&str>, delivery: &Value
) -> bool {
    let required = stage.map(|s| s.eq_ignore_ascii_case("required")).unwrap_or(false);
    let attached = stage == Some("evidence_attached");
    let enabled = delivery["retrievalFundingControl"].as_bool() == Some(true);
    if enabled {
        model.map(|m| !m.trim().is_empty() && m!="lfm3:1.7b")
            .unwrap_or(false) && (required || attached)
    } else {
        !required
    }
}

impl FundingSessionV176 {
    pub(crate) fn start(task: &TaskEnvelope, base: &str, auth: &LocalAuth, hardware: &str,
        execution: &str, node_stop: &Arc<AtomicBool>, ceiling_ms: Option<u64>) -> Result<Self,String> {
        let d=task.dynamic_funding_v176.as_ref().ok_or("funding_delivery_missing")?;
        if !node_enabled(task.selected_model.as_deref()) ||
            d["protocol"].as_str()!=Some(PROTOCOL) || d["status"].as_str()!=Some("funded") ||
            task.max_output_tokens.is_some() || task.execution_contract.as_ref().map(|c|c.output_token_budget.is_some() || c.progress_reporting!=Some(true)).unwrap_or(true) ||
            task.selected_model.as_deref().map(|s|s=="lfm3:1.7b").unwrap_or(true) || task.company_model.is_some() ||
            task.vision_input.is_some() || !retrieval_delivery_compatible_v176(
                task.tool_contract.as_ref().and_then(|c|c.stage.as_deref()),
                task.selected_model.as_deref(), d) {
            return Err("funding_delivery_not_supported".into());
        }
        let grant=d["authorizedOutputBillingTokens"].as_u64().filter(|n|*n<=u64::MAX/4).ok_or("funding_initial_grant_invalid")?;
        let interval=d["progressIntervalMs"].as_u64().filter(|n|(100..=60_000).contains(n)).ok_or("funding_progress_interval_invalid")?;
        let auth_client=SupabaseAuthClient::from_env()?;
        let http=Client::builder().timeout(Duration::from_secs(15)).build().map_err(|_|"funding_http_client_failed".to_string())?;
        let mut auth=auth.clone(); let task_id=task.task_id.clone(); let hardware=hardware.to_owned(); let exec=execution.to_owned();
        Self::start_with(base,grant,interval,execution,node_stop.clone(),ceiling_ms.unwrap_or(900_000),move |frame| {
            let body=frame.body(&task_id,&auth.provider_email,&hardware,&exec);
            send_frame(&http,&auth_client,&mut auth,&body)
        })
    }

    fn start_with<F>(base: &str, grant: u64, interval_ms: u64, execution: &str, node_stop: Arc<AtomicBool>, ceiling_ms: u64,
        mut send: F) -> Result<Self,String>
    where F: FnMut(&Frame)->Result<Value,SendError>+Send+'static {
        let local=local_client()?; probe(&local,base)?;
        let started=Instant::now();
        let deadline=started+Duration::from_millis(ceiling_ms.max(1));
        let mut state=Shared::new(grant);
        // One admission handshake before inference. A lost response is replayed with the same sequence.
        let first=Frame{sequence:1,usage:Usage::default(),elapsed:0};
        let receipt=match send(&first) { Ok(v)=>Receipt::parse(&v,&first,execution)?,
            Err(SendError::Retry)=>Receipt::parse(&send(&first).map_err(|_|"funding_start_unconfirmed")?,&first,execution)?,
            Err(SendError::Fenced)=>return Err("funding_start_fenced".into()) };
        state.receipt(receipt);
        let initial_grant=state.grant;
        let shared=Arc::new(Mutex::new(state)); let done=Arc::new(AtomicBool::new(false));
        let id=uuid::Uuid::new_v4().simple().to_string();
        let base=base.to_owned(); let worker_id=id.clone(); let execution=execution.to_owned();
        let wshared=shared.clone(); let wdone=done.clone(); let wstop=node_stop.clone();
        let worker=thread::spawn(move || {
            let interval=Duration::from_millis(interval_ms); let mut last=Instant::now();
            let mut sequence=2_u64; let mut usage=Usage::default(); let mut outstanding:Option<Frame>=None;
            let mut first_output=false; let mut last_pause=false; let mut final_reported=false; let mut local_grant=initial_grant;
            let mut previous_state:Option<(FinancialState,bool,bool)>=None;
            while !wdone.load(Ordering::Acquire) && !wstop.load(Ordering::Acquire) {
                if Instant::now()>=deadline {
                    let mut s=wshared.lock().unwrap();
                    s.fatal=Some(if s.reload && s.financial==FinancialState::Limited {"dynamic_funding_reload_required"} else {"dynamic_funding_execution_deadline"}.into());
                    break;
                }
                let (grant,final_usage)={let s=wshared.lock().unwrap();(s.grant,s.final_usage)};
                match runtime_usage(&local,&base,&worker_id,grant) {
                    Ok(Some(r))=>{
                        if r.usage.raw<usage.raw {
                            wshared.lock().unwrap().fatal=Some("funding_runtime_usage_regressed".into());break;
                        }
                        usage=r.usage;local_grant=r.grant;
                        let mut s=wshared.lock().unwrap(); s.observed_raw=s.observed_raw.max(usage.raw);s.paused=r.paused;s.saw_pause|=r.paused;
                    },
                    Ok(None)=>{}, // slot is not assigned yet, or has naturally finished
                    Err(_)=>{ /* retain the last confirmed grant; do not classify as exhausted */ }
                }
                let (paused,financial,reload)={let s=wshared.lock().unwrap();(s.paused,s.financial,s.reload)};
                let key=(financial,paused,reload);
                if previous_state!=Some(key) {
                    println!("DYNAMIC_FUNDING_V176 STATE={financial:?} RUNTIME_WAIT={paused} RELOAD_REQUIRED={}",paused && reload);
                    previous_state=Some(key);
                }
                if let Some(f)=final_usage {
                    usage=Usage{raw:usage.raw.max(f.raw),billing:f.billing};
                    if f.tokens()<=grant { break; }
                }
                let due=last.elapsed()>=interval || (!first_output && usage.raw>0) || (paused && !last_pause) || (final_usage.is_some() && !final_reported);
                if outstanding.is_none() && due {
                    outstanding=Some(Frame{sequence,usage,elapsed:started.elapsed().as_millis().min(u64::MAX as u128) as u64});
                }
                if let Some(frame)=outstanding.as_ref() {
                    match send(frame) {
                        Ok(v)=> match Receipt::parse(&v,frame,&execution) {
                            Ok(r)=> {
                                let mut s=wshared.lock().unwrap();s.receipt(r);drop(s);
                                sequence+=1;first_output|=frame.usage.raw>0;last_pause=paused;
                                final_reported |= final_usage.map(|f|f.billing==frame.usage.billing).unwrap_or(false);
                                last=Instant::now();outstanding=None;
                            },
                            Err(e)=>{wshared.lock().unwrap().fatal=Some(e);break;}
                        },
                        Err(SendError::Retry)=>{wshared.lock().unwrap().unconfirmed();},
                        Err(SendError::Fenced)=>{wshared.lock().unwrap().fatal=Some("dynamic_funding_progress_fenced".into());break;}
                    }
                }
                // Apply a confirmed extension immediately, without waiting for another progress period.
                let new_grant=wshared.lock().unwrap().grant;
                if new_grant>local_grant {
                    if let Ok(Some(r))=runtime_usage(&local,&base,&worker_id,new_grant) { local_grant=r.grant; }
                }
                // Polling continues when no SSE chunks arrive, including genuine exhaustion.
                thread::sleep(if outstanding.is_some() {Duration::from_millis(500)} else {Duration::from_millis(250)});
            }
        });
        Ok(Self{id,initial_grant,shared,done,node_stop,worker:Some(worker),deadline})
    }

    pub(crate) fn request(&self) -> Value { json!({"id":self.id,"authorizedOutputBillingTokens":self.initial_grant}) }
    pub(crate) fn remaining_ms(&self) -> Result<u64,String> {
        self.check()?;
        Ok(self.deadline.saturating_duration_since(Instant::now())
            .as_millis().min(u64::MAX as u128).max(1) as u64)
    }

    pub(crate) fn check(&self) -> Result<(),String> {
        if self.node_stop.load(Ordering::Acquire) {return Err("dynamic_funding_node_stopping".into());}
        let s=self.shared.lock().unwrap();
        if let Some(e)=&s.fatal {return Err(e.clone());}
        if Instant::now()>=self.deadline {return Err("dynamic_funding_execution_deadline".into());}
        Ok(())
    }
    pub(crate) fn finish(&self, canonical: &str) -> Result<(),String> {
        let f=Usage::final_output(canonical);
        { let mut s=self.shared.lock().unwrap();s.final_usage=Some(f); }
        // Canonical encoding may differ from the runtime's conservative estimate. Never
        // submit a successful result with a locally unconfirmed final authorization.
        loop {
            self.check()?;
            if self.shared.lock().unwrap().grant>=f.tokens() {return Ok(());}
            thread::sleep(Duration::from_millis(250));
        }
    }
}
impl Drop for FundingSessionV176 {
    fn drop(&mut self) {
        self.done.store(true,Ordering::Release);
        if let Some(w)=self.worker.take() {let _=w.join();}
    }
}

fn funding_opt_in_v176(model: Option<&str>, setting: Option<&str>) -> bool {
    match setting {
        Some("1") => true,
        None => model.map(|m|
            !m.trim().is_empty() && m != "lfm3:1.7b"
        ).unwrap_or(false),
        _ => false,
    }
}

fn node_enabled(model: Option<&str>) -> bool {
    cfg!(any(target_os="macos",target_os="windows",target_os="linux")) &&
    funding_opt_in_v176(model,
        std::env::var("SWARM_V176_DYNAMIC_FUNDING_NODE_ENABLE").ok().as_deref()) &&
    std::env::var("EDGESWARM_LLAMA_BASE_URL").ok().filter(|s|!s.trim().is_empty()).is_none()
}



pub(crate) fn runtime_context_metadata_v176(
    h: &crate::core::production_heartbeat::ProductionHeartbeatV1
) -> Option<Value> {
    if !cfg!(any(target_os="macos",target_os="windows",target_os="linux"))
        || !matches!(h.app_version.as_str(),"1.7.6" | "1.7.7")
        || h.status!="online" || h.model_status!="ready"
        || h.runtime.as_deref()!=Some("llama.cpp")
        || h.metadata.company_model_v1.ready
        || h.concurrency_limit==0
        || std::env::var("EDGESWARM_LLAMA_BASE_URL").ok()
            .filter(|s| !s.trim().is_empty()).is_some()
    { return None; }

    let model=h.model_id.as_deref()?;
    let definition=crate::core::signed_model_registry::
        signed_model_definition_v171(model)?;

    if definition.runtime!="llama.cpp" { return None; }

    let certified=h.metadata.model_capacity_v1.iter().any(|m|
        m.selected_model==model
        && m.model_id==definition.artifact_id
        && m.capability==definition.capability
        && m.status=="ready"
        && m.capacity_status==crate::core::capacity::CapacityStatus::Certified
        && m.certified_concurrency.unwrap_or(0)>=h.concurrency_limit
    );
    if !certified { return None; }

    let base=format!("http://{}:{}",
        crate::runtime::llama_process::LLAMA_DEFAULT_HOST,
        crate::runtime::llama_process::LLAMA_DEFAULT_PORT);
    let http=local_client().ok()?;
    let props:Value=http.get(format!("{base}/props")).send().ok()?
        .error_for_status().ok()?.json().ok()?;

    let path=std::path::Path::new(props["model_path"].as_str()?);
    let filename=path.file_name()?.to_str()?;
    let artifact=definition.artifacts.iter().find(|a|
        a.artifact_role=="model" && a.filename==filename)?;

    let slots:Value=http.get(format!("{base}/slots")).send().ok()?
        .error_for_status().ok()?.json().ok()?;
    let slots=slots.as_array()?;
    if slots.len()<h.concurrency_limit as usize
        || props["total_slots"].as_u64()? != slots.len() as u64
    { return None; }

    let contexts=slots.iter().map(|s|s["n_ctx"].as_u64())
        .collect::<Option<Vec<u64>>>()?;
    if contexts.iter().any(|n| !(256..=1048576).contains(n)) {
        return None;
    }
    let context=*contexts.iter().min()?;

    Some(json!({
        "version":"managed-runtime-context-v176",
        "modelId":model,
        "modelArtifactId":definition.artifact_id,
        "artifactSha256":artifact.sha256,
        "runtimeSha256":h.metadata.runtime_sha256.as_ref()?,
        "contextWindowTokens":context,
        "slotCount":slots.len(),
        "certifiedContextEvidence":false
    }))
}

pub(crate) fn heartbeat_metadata_v176(heartbeat: &crate::core::production_heartbeat::ProductionHeartbeatV1) -> Option<Value> {
    // Explicit local opt-in + actual runtime probe. Disabled by default, including release builds.
    if !node_enabled(heartbeat.model_id.as_deref()) || !matches!(heartbeat.app_version.as_str(),"1.7.6" | "1.7.7") || heartbeat.status!="online" || heartbeat.model_status!="ready" ||
        heartbeat.model_id.as_deref().map(|s|s=="lfm3:1.7b").unwrap_or(true) ||
        heartbeat.metadata.company_model_v1.ready {return None;}
    let base=format!("http://{}:{}",crate::runtime::llama_process::LLAMA_DEFAULT_HOST,crate::runtime::llama_process::LLAMA_DEFAULT_PORT);
    let http=local_client().ok()?;probe(&http,&base).ok()?;
    Some(json!({"protocol":PROTOCOL,"runtimeFundingControl":true,
        "retrievalFundingControl":heartbeat.metadata.runtime_context_v176.is_some()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reply(f:&Frame,e:&str,status:&str,grant:Option<u64>,reload:bool)->Value {
        json!({"ok":true,"sequence":f.sequence,"executionId":e,"event":f.event(),"dynamicFundingV176":{
            "protocol":PROTOCOL,"status":status,"sequence":f.sequence,"executionId":e,"authorizedOutputBillingTokens":grant,"reloadRequired":reload}})
    }
    #[test]
    fn v176_financial_receipts_fence_execution_sequence_and_keep_confirmed_grant() {
        let f=Frame{sequence:2,usage:Usage{raw:12,billing:27},elapsed:100};let mut s=Shared::new(100);
        let v=reply(&f,"x","funded",Some(200),false);s.receipt(Receipt::parse(&v,&f,"x").unwrap());assert_eq!(s.grant,200);
        let v=reply(&f,"x","funding_limited",Some(120),false);s.receipt(Receipt::parse(&v,&f,"x").unwrap());assert_eq!(s.grant,200);assert!(!s.reload);
        let v=reply(&f,"x","funding_unconfirmed",Some(5000),true);s.receipt(Receipt::parse(&v,&f,"x").unwrap());assert_eq!(s.grant,200);assert!(!s.reload);
        let v=reply(&f,"x","funding_limited",Some(200),true);s.receipt(Receipt::parse(&v,&f,"x").unwrap());assert!(s.reload);
        assert!(Receipt::parse(&v,&f,"wrong").is_err());let mut wrong=f.clone();wrong.sequence=3;assert!(Receipt::parse(&v,&wrong,"x").is_err());
        assert_eq!(Usage::final_output("😀").billing,2);assert_eq!(Usage{raw:1,billing:5}.tokens(),2);
    }
    #[test]
    fn v176_progress_body_keeps_legacy_identity_and_billing_usage_separate() {
        let f=Frame{sequence:3,usage:Usage{raw:8,billing:25},elapsed:123};let v=f.body(&json!(42),"test@example.invalid","device","exec");
        assert_eq!(v["outputChars"],8);assert_eq!(v["outputBillingCharsV176"],25);assert_eq!(v["event"],"generation.progress");
        assert!(loopback("http://127.0.0.1:18082"));assert!(!loopback("http://example.com:18082"));assert!(!loopback("http://127.0.0.1:18082@evil.example"));
    }
    #[test]
    #[ignore = "requires the isolated funded Metal runtime on port 18082"]
    fn v176_live_runtime_funding_bridge() {
        let base="http://127.0.0.1:18082";let exec="fixture-v176";
        let mut boundary_seen:Option<Instant>=None;let mut outage=false;let mut last_raw=0;let mut last_seq=0;
        let session=FundingSessionV176::start_with(base,8,500,exec,Arc::new(AtomicBool::new(false)),180_000,move |f| {
            assert!(f.sequence>last_seq);assert!(f.usage.raw>=last_raw);last_seq=f.sequence;last_raw=f.usage.raw;
            if f.usage.tokens()>8 {
                if !outage {outage=true;return Ok(reply(f,exec,"funding_unconfirmed",None,false));}
                let since=*boundary_seen.get_or_insert_with(Instant::now);
                if since.elapsed()>=Duration::from_secs(2) {return Ok(reply(f,exec,"funded",Some(8192),false));}
                return Ok(reply(f,exec,"funding_limited",Some(8),true));
            }
            Ok(reply(f,exec,"funded",Some(8),false))
        }).unwrap();
        let llama=crate::core::production_inference::ProductionLlamaClient::new(base).unwrap();
        llama.health_check().unwrap();
        let prompt="Write a numbered list of 2000 short sentences about the planets. Include a separate sentence for each number, starting at 1. Continue as long as possible.";
        let mut content=String::new();
        let result={
            let mut collect=|delta:&str|content.push_str(delta);
            llama.execute_customer_v176(prompt,prompt,None,true,Some(180_000),None,Some(&mut collect),Some(&session)).unwrap()
        };
        assert!(matches!(result.finish_reason.as_deref(),Some("stop" | "length")));
        assert_eq!(result.max_tokens,0);
        assert!(content.len()>40);
        session.finish(&result.ai_output).unwrap();
        let s=session.shared.lock().unwrap();assert!(s.saw_pause);assert!(s.saw_reload);assert!(s.saw_unconfirmed);assert_eq!(s.grant,8192);
        println!("MAC_CLIENT_RUNTIME_FUNDING_BRIDGE=PASS");
    }
}
