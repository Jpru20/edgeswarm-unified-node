#!/usr/bin/env python3
"""Windows local-only native funding certification; no Swarm or database calls."""
import argparse, json, os, socket, subprocess, tempfile, threading, time, urllib.error, urllib.request, uuid
from pathlib import Path

parser=argparse.ArgumentParser()
parser.add_argument('runtime', type=Path)
parser.add_argument('model', type=Path)
parser.add_argument('--gpu-layers', type=int, default=0)
parser.add_argument('--context', type=int, choices=[512,1024,2048,4096], default=512)
parser.add_argument('--diagnose-output', action='store_true', help='Also test an ordinary natural completion and report stream field counts')
args=parser.parse_args()
runtime=args.runtime.resolve()
model=args.model.resolve()
assert runtime.is_file(), 'TEST_RUNTIME_MISSING'
assert model.is_file(), 'TEST_MODEL_MISSING'
assert args.gpu_layers >= 0, 'INVALID_GPU_LAYERS'
print('TEST_MODEL='+model.name,flush=True)
print('TEST_GPU_LAYERS='+str(args.gpu_layers),flush=True)
print('TEST_CONTEXT='+str(args.context),flush=True)
for port in (18081,18082):
    with socket.socket() as s:
        s.settimeout(1)
        assert s.connect_ex(('127.0.0.1',port))!=0, f'PORT_IN_USE={port}'
base='http://127.0.0.1:18082'
fd, log_name=tempfile.mkstemp(prefix='swarm-v176-win-funding-',suffix='.log')
os.close(fd)
log_path=Path(log_name)
print('RUNTIME_LOG='+str(log_path),flush=True)
log=log_path.open('wb')
# Short diagnostic context only. Production model capacity is not changed.
p=subprocess.Popen([str(runtime),'-m',str(model),'--host','127.0.0.1','--port','18082',
    '--ctx-size',str(args.context),'--parallel','2','--no-context-shift','--cache-ram','0',
    '--no-cache-idle-slots','--no-cache-prompt','--slot-prompt-similarity','0',
    '--n-gpu-layers',str(args.gpu_layers)],stdout=log,stderr=log,cwd=str(runtime.parent))
opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))

def open_request(path,body=None,timeout=30):
    return opener.open(urllib.request.Request(base+path,data=None if body is None else json.dumps(body).encode(),
        headers={'Content-Type':'application/json'}),timeout=timeout)

def call(path,body=None,timeout=30):
    with open_request(path,body,timeout) as r:return json.load(r)

CONTROL='/v1/chat/completions/control'
def status(id,grant=None):
    body={'action':'swarm_funding_v176','id':id}
    if grant is not None:body['authorizedOutputBillingTokens']=grant
    r=call(CONTROL,body)
    if r.get('success') is not True:return None
    s=r.get('swarmFundingV176')
    assert s and s['protocol']=='dynamic-funding-v176'
    return s

class Stream:
    def __init__(self,grant=None,prompt=None,ignore_eos=True,label='funding'):
        self.id=uuid.uuid4().hex
        self.done=threading.Event();self.parts=[];self.finish=None;self.usage=None;self.error=None
        self.label=label;self.delta_fields=set();self.reasoning_chars=0;self.delta_frames=0;self.saw_done=False
        body={'model':'local-model','messages':[{'role':'user','content':
            prompt if prompt is not None else 'Write a continuous numbered list of integers, starting at 1. Do not stop early.'}],
            'max_tokens':-1,'temperature':0,'seed':123,'ignore_eos':ignore_eos,'stream':True,
            'stream_options':{'include_usage':True},'cache_prompt':False}
        if grant is not None:body['swarmFundingV176']={'id':self.id,'authorizedOutputBillingTokens':grant}
        def worker():
            try:
                with open_request('/v1/chat/completions',body,timeout=300) as response:
                    for line in response:
                        line=line.decode('utf-8').strip()
                        if not line.startswith('data:'):continue
                        data=line[5:].strip()
                        if data=='[DONE]':
                            self.saw_done=True
                            break
                        r=json.loads(data)
                        if r.get('error'):raise RuntimeError(str(r['error']))
                        for c in r.get('choices',[]):
                            delta=c.get('delta') or {}
                            self.delta_fields.update(delta.keys())
                            self.delta_frames+=1
                            v=delta.get('content')
                            if isinstance(v,str) and v:self.parts.append(v)
                            for key in ('reasoning_content','reasoning'):
                                reasoning=delta.get(key)
                                if isinstance(reasoning,str):self.reasoning_chars+=len(reasoning)
                            if c.get('finish_reason'):self.finish=c['finish_reason']
                        if r.get('usage'):self.usage=r['usage']
            except BaseException as e:self.error=e
            finally:self.done.set()
        self.thread=threading.Thread(target=worker,daemon=True);self.thread.start()
    def check(self):
        if self.error:raise RuntimeError('TEST_STREAM_ERROR') from self.error
    def report(self):
        print('STREAM_DIAGNOSTIC='+json.dumps({'label':self.label,'finish':self.finish,'usage':self.usage,'visible_chars':sum(map(len,self.parts)),'reasoning_chars':self.reasoning_chars,'delta_fields':sorted(self.delta_fields),'delta_frames':self.delta_frames,'saw_done':self.saw_done},ensure_ascii=True),flush=True)
    def wait(self,seconds=300):
        assert self.done.wait(seconds),'STREAM_DID_NOT_FINISH'
        self.check()
        if args.diagnose_output or not self.parts:self.report()
        assert self.finish=='length',f'UNEXPECTED_FINISH={self.finish}'
        assert self.parts,'EMPTY_STREAM'
        assert self.usage and self.usage['completion_tokens']>0,'RUNTIME_USAGE_MISSING'
        return ''.join(self.parts)

def wait_status(stream,predicate,timeout=120):
    until=time.monotonic()+timeout
    while time.monotonic()<until:
        stream.check()
        assert not stream.done.is_set(),'STREAM_FINISHED_BEFORE_REQUIRED_STATE'
        s=status(stream.id)
        if s and predicate(s):return s
        time.sleep(.05)
    raise AssertionError('FUNDING_STATE_NOT_REACHED')

try:
    until=time.monotonic()+300
    while time.monotonic()<until:
        if p.poll() is not None:raise RuntimeError('RUNTIME_EXITED_BEFORE_HEALTH')
        try:
            if call('/health',timeout=2).get('status')=='ok':break
        except (OSError,urllib.error.URLError):pass
        time.sleep(1)
    else:raise RuntimeError('RUNTIME_NOT_READY')
    probe=call(CONTROL,{'action':'swarm_funding_probe_v176'})
    assert probe=={'success':True,'protocol':'dynamic-funding-v176'},probe
    print('RUNTIME_FUNDING_CONTROL_PROBE=PASS',flush=True)

    # Reject malformed grants before any decoding. The valid path is tested below.
    def expect_bad_request(path,body):
        try:
            call(path,body)
        except urllib.error.HTTPError as e:
            assert e.code==400, (e.code,e.read().decode(errors='replace'))
        else:
            raise AssertionError('INVALID_FUNDING_GRANT_ACCEPTED')
    for grant in (-1,1.5,True,"8",2**62):
        expect_bad_request('/v1/chat/completions',{
            'messages':[{'role':'user','content':'Test'}],
            'max_tokens':-1,'stream':True,
            'swarmFundingV176':{'id':uuid.uuid4().hex,'authorizedOutputBillingTokens':grant}})
        expect_bad_request(CONTROL,{
            'action':'swarm_funding_v176','id':uuid.uuid4().hex,
            'authorizedOutputBillingTokens':grant})
    print('INVALID_FUNDING_GRANTS_REJECTED=PASS',flush=True)

    # Diagnostic only: distinguish a normal answer from forced continuation.
    baseline_ok=True
    if args.diagnose_output:
        normal=Stream(prompt='What is 2 plus 2? Answer in one short sentence.',ignore_eos=False,label='natural_completion_unfunded')
        assert normal.done.wait(300),'NATURAL_STREAM_DID_NOT_FINISH'
        normal.check()
        normal.report()
        baseline_ok=(normal.saw_done and normal.finish=='stop' and bool(''.join(normal.parts).strip()) and '4' in ''.join(normal.parts) and bool(normal.usage and normal.usage.get('completion_tokens',0)>0))
        print('NATURAL_COMPLETION_BASELINE='+('PASS' if baseline_ok else 'FAIL'),flush=True)

    # Early grant update: initial allowance is large enough to enter active generation.
    # No inference output limit is used; ignore_eos is diagnostic only.
    early=Stream(128,label='funded_proactive_extension')
    before=wait_status(early,lambda s:s['generatedTokens']>=2)
    assert not before['fundingPaused'] and before['pauseCount']==0,before
    extended=status(early.id,8192)
    assert extended and not extended['fundingPaused'] and extended['pauseCount']==0,extended
    assert extended['authorizedOutputBillingTokens']==8192,extended
    stale=status(early.id,128)
    assert stale and stale['authorizedOutputBillingTokens']==8192,stale
    early.wait()
    print('PROACTIVE_EXTENSION_DURING_GENERATION=PASS',flush=True)
    print('STALE_LOCAL_GRANT_CANNOT_REVOKE=PASS',flush=True)

    # Exhaust actual authorization. The held token must not lead to more sampling.
    blocked=Stream(8,label='funded_exhaustion_resume')
    first=wait_status(blocked,lambda s:s['fundingPaused'])
    assert first['pendingToken'] and first['pauseCount']==1,first
    assert first['outputBillingChars']>first['authorizedOutputBillingTokens']*4,first
    time.sleep(1.0)
    still=status(blocked.id)
    assert still and still['fundingPaused'],still
    assert still['generatedTokens']==first['generatedTokens'] and still['outputChars']==first['outputChars'],(first,still)
    assert not blocked.done.is_set()
    print('EXHAUSTION_HOLDS_SAME_SAMPLED_TOKEN=PASS',flush=True)
    assert status(uuid.uuid4().hex,8192) is None
    print('CROSS_COMPLETION_CONTROL_REJECTED=PASS',flush=True)

    # Other slots must keep working while the first request waits for funding.
    other=call('/v1/chat/completions',{
        'messages':[{'role':'user','content':'List integers starting at one.'}],
        'max_tokens':24,'ignore_eos':True,'temperature':0,'stream':False,'cache_prompt':False},timeout=180)
    assert other['choices'][0]['message']['content']
    assert other['usage']['completion_tokens']>0
    same=status(blocked.id)
    assert same and same['fundingPaused'] and same['generatedTokens']==first['generatedTokens'],same
    print('OTHER_SLOT_GENERATES_WHILE_FUNDS_EXHAUSTED=PASS',flush=True)

    restored=status(blocked.id,8192)
    assert restored and not restored['fundingPaused'] and restored['pendingToken'],restored
    assert restored['pauseCount']==1,restored
    # Retain the original HTTP request and context; no restart or prompt replay.
    text=blocked.wait()
    assert blocked.usage['completion_tokens']>first['generatedTokens']
    assert text
    print('SAME_EXECUTION_RESUMES_AFTER_GRANT=PASS',flush=True)
    print('PHYSICAL_CONTEXT_EXHAUSTION_REMAINS_LENGTH=PASS',flush=True)
    print('UNFUNDED_LEGACY_REQUEST_UNCHANGED=PASS',flush=True)
    # The runtime log should show exactly one pause and one resume across the two funded requests.
    log.flush()
    lines=log_path.read_text(encoding='utf-8',errors='replace')
    assert lines.count('SWARM_FUNDING_BOUNDARY_WAIT_V176')==1,'UNEXPECTED_ROUTINE_FUNDING_PAUSE'
    assert lines.count('SWARM_FUNDING_RESUMED_V176')==1,'UNEXPECTED_RESUME_COUNT'
    print('NO_ROUTINE_PAUSES_WHILE_AUTHORIZED=PASS',flush=True)
    assert baseline_ok,'NATURAL_COMPLETION_BASELINE_FAILED'
finally:
    if p.poll() is None:
        p.terminate()
        try:p.wait(timeout=30)
        except subprocess.TimeoutExpired:p.kill();p.wait()
    log.close()
