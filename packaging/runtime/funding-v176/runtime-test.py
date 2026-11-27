#!/usr/bin/env python3
"""Local-only live runtime gate. Never connects to Swarm or the database."""
import json, os, socket, subprocess, sys, tempfile, threading, time, urllib.error, urllib.request, uuid
from pathlib import Path

runtime=Path(sys.argv[1]).resolve()
model=Path.home()/'Library/Application Support/EdgeSwarm/models/Qwen2.5-7B-Instruct-Q4_K_M.gguf'
assert model.is_file(), 'TEST_MODEL_MISSING'
for port in (18081,18082):
    with socket.socket() as s:
        s.settimeout(1)
        assert s.connect_ex(('127.0.0.1',port))!=0, f'PORT_IN_USE={port}'
base='http://127.0.0.1:18082'
log=tempfile.NamedTemporaryFile(prefix='swarm-v176-funding-',suffix='.log',delete=False)
print('RUNTIME_LOG='+log.name,flush=True)
# Short diagnostic context only. Production model capacity is not changed.
p=subprocess.Popen([str(runtime),'-m',str(model),'--host','127.0.0.1','--port','18082',
    '--ctx-size','1024','--parallel','2','--no-context-shift','--cache-ram','0',
    '--no-cache-idle-slots','--no-cache-prompt','--slot-prompt-similarity','0',
    '--n-gpu-layers','999'],stdout=log,stderr=log)
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
    def __init__(self,grant=None):
        self.id=uuid.uuid4().hex
        self.done=threading.Event();self.parts=[];self.finish=None;self.usage=None;self.error=None
        body={'model':'local-model','messages':[{'role':'user','content':
            'Write a continuous numbered list of integers, starting at 1. Do not stop early.'}],
            'max_tokens':-1,'temperature':0,'seed':123,'ignore_eos':True,'stream':True,
            'stream_options':{'include_usage':True},'cache_prompt':False}
        if grant is not None:body['swarmFundingV176']={'id':self.id,'authorizedOutputBillingTokens':grant}
        def worker():
            try:
                with open_request('/v1/chat/completions',body,timeout=180) as response:
                    for line in response:
                        line=line.decode('utf-8').strip()
                        if not line.startswith('data:'):continue
                        data=line[5:].strip()
                        if data=='[DONE]':break
                        r=json.loads(data)
                        if r.get('error'):raise RuntimeError(str(r['error']))
                        for c in r.get('choices',[]):
                            v=c.get('delta',{}).get('content')
                            if v:self.parts.append(v)
                            if c.get('finish_reason'):self.finish=c['finish_reason']
                        if r.get('usage'):self.usage=r['usage']
            except BaseException as e:self.error=e
            finally:self.done.set()
        self.thread=threading.Thread(target=worker,daemon=True);self.thread.start()
    def check(self):
        if self.error:raise RuntimeError('TEST_STREAM_ERROR') from self.error
    def wait(self,seconds=180):
        assert self.done.wait(seconds),'STREAM_DID_NOT_FINISH'
        self.check()
        assert self.finish=='length',f'UNEXPECTED_FINISH={self.finish}'
        assert self.parts,'EMPTY_STREAM'
        assert self.usage and self.usage['completion_tokens']>0,'RUNTIME_USAGE_MISSING'
        return ''.join(self.parts)

def wait_status(stream,predicate,timeout=60):
    until=time.monotonic()+timeout
    while time.monotonic()<until:
        stream.check()
        assert not stream.done.is_set(),'STREAM_FINISHED_BEFORE_REQUIRED_STATE'
        s=status(stream.id)
        if s and predicate(s):return s
        time.sleep(.05)
    raise AssertionError('FUNDING_STATE_NOT_REACHED')

try:
    until=time.monotonic()+180
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

    # Early grant update: initial allowance is large enough to enter active generation.
    # No inference output limit is used; ignore_eos is diagnostic only.
    early=Stream(128)
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
    blocked=Stream(8)
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
        'max_tokens':24,'ignore_eos':True,'temperature':0,'stream':False,'cache_prompt':False},timeout=90)
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
    lines=Path(log.name).read_text(errors='replace')
    assert lines.count('SWARM_FUNDING_BOUNDARY_WAIT_V176')==1,'UNEXPECTED_ROUTINE_FUNDING_PAUSE'
    assert lines.count('SWARM_FUNDING_RESUMED_V176')==1,'UNEXPECTED_RESUME_COUNT'
    print('NO_ROUTINE_PAUSES_WHILE_AUTHORIZED=PASS',flush=True)
finally:
    if p.poll() is None:
        p.terminate()
        try:p.wait(timeout=30)
        except subprocess.TimeoutExpired:p.kill();p.wait()
    log.close()
