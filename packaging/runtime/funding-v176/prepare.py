#!/usr/bin/env python3
"""Stage and build an isolated, source-fenced Mac runtime. Does not install it."""
from pathlib import Path
import hashlib, os, platform, re, shutil, subprocess, tempfile

KIT = Path(__file__).resolve().parent
HOME = Path.home()
ORIGINAL = HOME / 'edgeswarm-runtime-build/llama.cpp-a3b1effcda84caeb180427b1346d0212841418f6'
SERVER = Path('tools/server')
EXPECTED = {
    SERVER/'server-context.cpp': '21ff783941382346293429b30e2ff1501516a491',
    SERVER/'server-task.h': '25ff015122488e70bbd4b4e18fe3ef57dc63d4cd',
}

def blobsha(p):
    data=p.read_bytes()
    return hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()

def one(s,a,b):
    n=s.count(a)
    if n!=1: raise RuntimeError('RUNTIME_PATCH_ANCHOR_CONFLICT='+repr(a[:120])+' matches='+str(n))
    return s.replace(a,b,1)

def patch_task(s):
    s=one(s, '    std::string        control_cmpl_id;\n', '''    std::string        control_cmpl_id;

    // SWARM_FUNDING_V176: server-authorized financial runway, not n_predict.
    std::string        swarm_funding_id_v176;
    uint64_t           swarm_funding_initial_chars_v176 = 0;
    bool               swarm_funding_update_v176 = false;
    uint64_t           swarm_funding_control_chars_v176 = 0;
''')
    s=one(s, '    std::string message; // optional detail when success is false\n', '''    std::string message; // optional detail when success is false
    json swarm_funding_v176 = nullptr;
''')
    a='''        json out = json { { "success", success } };
        if (!message.empty()) {'''
    b='''        json out = json { { "success", success } };
        if (!swarm_funding_v176.is_null()) {
            out["swarmFundingV176"] = swarm_funding_v176;
        }
        if (!message.empty()) {'''
    return one(s,a,b)

def patch_context(s):
    s=one(s,'#include "server-task.h"\n','#include "server-task.h"\n#include "swarm-funding-v176.h"\n#include <optional>\n')
    s=one(s,'    int32_t n_predict_max = -1;\n','''    int32_t n_predict_max = -1;

    // One already-sampled token can be held at the actual funding boundary.
    // It is accepted by process_token exactly once, after funds are confirmed.
    uint64_t swarm_funding_chars_v176 = 0;
    uint64_t swarm_funding_pause_count_v176 = 0;
    swarm_funding_meter_v176 swarm_funding_meter;
    swarm_funding_meter_v176 swarm_funding_pending_meter;
    std::optional<completion_token_output> swarm_funding_pending;

    bool swarm_funding_blocked_v176() const {
        return swarm_funding_pending.has_value() &&
            swarm_funding_pending_meter.billing_chars() > swarm_funding_chars_v176;
    }
''')
    s=one(s,'        n_predict_max = -1;\n','''        n_predict_max = -1;
        swarm_funding_chars_v176 = 0;
        swarm_funding_pause_count_v176 = 0;
        swarm_funding_meter = {};
        swarm_funding_pending_meter = {};
        swarm_funding_pending.reset();
''')
    s=one(s,'    server_slot * get_available_slot(const server_task & task) {','''    server_slot * get_swarm_funding_slot_v176(const std::string & id) {
        if (!swarm_funding_id_valid_v176(id)) return nullptr;
        for (server_slot & slot : slots) {
            if (slot.is_processing() && slot.task &&
                slot.task->params.swarm_funding_id_v176 == id) return &slot;
        }
        return nullptr;
    }

    server_slot * get_available_slot(const server_task & task) {''')
    s=one(s,'    bool launch_slot_with_task(server_slot & slot, server_task && task) {','''    bool launch_slot_with_task(server_slot & slot, server_task && task) {
        if (!task.params.swarm_funding_id_v176.empty()) {
            // Certified first path: single text stream, ordinary sampling.
            // Legacy, LFM3 and nonfunded requests never enter this gate.
            if (spec || mctx || params_base.ctx_shift || params_base.n_predict != -1 ||
                task.params.n_predict != -1 || task.params.n_cmpl != 1 ||
                !task.params.stream || task.is_parent() || task.is_child() ||
                get_swarm_funding_slot_v176(task.params.swarm_funding_id_v176)) {
                send_error(task, "funding_runtime_mode_unsupported", ERROR_TYPE_INVALID_REQUEST);
                return false;
            }
        }
''')
    s=one(s,'        slot.task = std::make_unique<const server_task>(std::move(task));\n','''        slot.swarm_funding_chars_v176 = task.params.swarm_funding_initial_chars_v176;
        slot.task = std::make_unique<const server_task>(std::move(task));
''')
    s=one(s,'    bool process_token(completion_token_output & result, server_slot & slot) {','''    bool swarm_funding_accept_v176(completion_token_output & result, server_slot & slot) {
        if (slot.task->params.swarm_funding_id_v176.empty()) return true;
        auto prospective = slot.swarm_funding_meter;
        prospective.append(result.text_to_send);
        if (prospective.billing_chars() <= slot.swarm_funding_chars_v176) {
            slot.swarm_funding_meter = std::move(prospective);
            return true;
        }
        GGML_ASSERT(!slot.swarm_funding_pending.has_value());
        slot.swarm_funding_pending_meter = std::move(prospective);
        slot.swarm_funding_pending = std::move(result);
        ++slot.swarm_funding_pause_count_v176;
        SLT_INF(slot, "%s", "SWARM_FUNDING_BOUNDARY_WAIT_V176\\n");
        return false;
    }

    bool process_token(completion_token_output & result, server_slot & slot) {''')
    s=one(s,'''                    server_slot * slot = get_slot_by_cmpl_id(task.params.control_cmpl_id);''','''                    if (task.params.control_action == "swarm_funding_v176") {
                        server_slot * funded = get_swarm_funding_slot_v176(task.params.control_cmpl_id);
                        if (!funded) {
                            res->success = false;
                            res->message = "no active funded completion";
                        } else {
                            // Grants only grow. Stale local acknowledgments cannot revoke a later grant.
                            if (task.params.swarm_funding_update_v176) {
                                funded->swarm_funding_chars_v176 = std::max(funded->swarm_funding_chars_v176,
                                    task.params.swarm_funding_control_chars_v176);
                            }
                            const auto & demand = funded->swarm_funding_pending.has_value()
                                ? funded->swarm_funding_pending_meter : funded->swarm_funding_meter;
                            res->success = true;
                            res->swarm_funding_v176 = json{
                                {"protocol", "dynamic-funding-v176"},
                                {"fundingPaused", funded->swarm_funding_blocked_v176()},
                                {"pendingToken", funded->swarm_funding_pending.has_value()},
                                {"pauseCount", funded->swarm_funding_pause_count_v176},
                                {"outputChars", demand.raw_bytes},
                                {"outputBillingChars", demand.billing_chars()},
                                {"authorizedOutputBillingTokens", funded->swarm_funding_chars_v176 / 4},
                                {"generatedTokens", funded->stats.n_gen}
                            };
                        }
                        queue_results.send(std::move(res));
                        break;
                    }

                    server_slot * slot = get_slot_by_cmpl_id(task.params.control_cmpl_id);''')
    s=one(s,'''            bool all_idle = true;

            for (auto & slot : slots) {
                if (slot.is_processing()) {
                    all_idle = false;
                    break;
                }
            }

            if (all_idle) {
                SRV_TRC("%s", "all slots are idle\\n");

                metrics_flush_idle();''','''            bool all_idle = true;
            bool funding_waiters = false;

            for (auto & slot : slots) {
                if (slot.is_processing()) {
                    if (slot.swarm_funding_blocked_v176()) {
                        funding_waiters = true;
                        continue;
                    }
                    all_idle = false;
                    break;
                }
            }

            if (all_idle) {
                // No busy spin when the only active slots await credits.
                // A local funding control task or another request wakes the queue.
                if (funding_waiters) return;
                SRV_TRC("%s", "all slots are idle\\n");

                metrics_flush_idle();''')
    s=one(s,'''    void pre_decode() {
        // apply context-shift if needed''','''    void pre_decode() {
        // Resume a held sampled token without resampling or replaying its prompt.
        // This runs only on the scheduler thread, before a new batch is built.
        iterate(slots, [&](server_slot & slot) {
            if (!slot.swarm_funding_pending.has_value() || slot.swarm_funding_blocked_v176()) return;
            auto result = std::move(*slot.swarm_funding_pending);
            slot.swarm_funding_pending.reset();
            slot.swarm_funding_meter = std::move(slot.swarm_funding_pending_meter);
            SLT_INF(slot, "%s", "SWARM_FUNDING_RESUMED_V176\\n");
            if (!process_token(result, slot)) {
                slot.print_timings();
                send_final_response(slot);
                slot.release();
            } else {
                slot.print_timings_tg();
            }
        });

        // apply context-shift if needed''')
    s=one(s,'            if (slot.state == SLOT_STATE_GENERATING && slot.prompt.n_tokens() + 1 >= slot.n_ctx) {','            if (slot.state == SLOT_STATE_GENERATING && !slot.swarm_funding_pending.has_value() && slot.prompt.n_tokens() + 1 >= slot.n_ctx) {')
    s=one(s,'''        // determine which slots are generating and drafting
        iterate(slots, [&](server_slot & slot) {
            if (slot.state != SLOT_STATE_GENERATING) {''','''        // determine which slots are generating and drafting
        iterate(slots, [&](server_slot & slot) {
            if (slot.state != SLOT_STATE_GENERATING || slot.swarm_funding_pending.has_value()) {''')
    # Do not allow a paused slot to become batch owner in the prompt loop either.
    s=one(s,'''                if (!slot.is_processing()) {
                    return;
                }

                // check if we can batch this slot with the previous one''','''                if (!slot.is_processing() || slot.swarm_funding_pending.has_value()) {
                    return;
                }

                // check if we can batch this slot with the previous one''')
    s=one(s,'''            if (!process_token(result, slot)) {
                // release slot because of stop condition''','''            if (!swarm_funding_accept_v176(result, slot)) {
                // Hold the single sampled token and its sampler state, not the whole runtime.
                return;
            }
            if (!process_token(result, slot)) {
                // release slot because of stop condition''')
    s=one(s,'''            task.params.message_spans = task.tokens.find_message_spans(delimiters);''','''            if (data.contains("swarmFundingV176")) {
                const auto & funding = data.at("swarmFundingV176");
                if (res_type != TASK_RESPONSE_TYPE_OAI_CHAT || !funding.is_object() ||
                    inputs.size() != 1 || !files.empty() || !task.params.stream ||
                    task.params.n_cmpl != 1 || task.params.n_predict != -1 ||
                    !funding.contains("id") || !funding.at("id").is_string() ||
                    !funding.contains("authorizedOutputBillingTokens") ||
                    !funding.at("authorizedOutputBillingTokens").is_number_unsigned()) {
                    throw std::runtime_error("invalid_swarm_funding_contract");
                }
                task.params.swarm_funding_id_v176 = funding.at("id").get<std::string>();
                if (!swarm_funding_id_valid_v176(task.params.swarm_funding_id_v176)) {
                    throw std::runtime_error("invalid_swarm_funding_identity");
                }
                task.params.swarm_funding_initial_chars_v176 = swarm_funding_chars_v176(
                    funding.at("authorizedOutputBillingTokens").get<uint64_t>());
            }

            task.params.message_spans = task.tokens.find_message_spans(delimiters);''')
    # Explicitly pass the private funding field through only the chat request parser.
    a=s.index('    this->post_chat_completions = [this]')
    b=s.index('    this->post_chat_completions_tok = [this]',a)
    s=s[:a]+one(s[a:b],'''            files);
        return handle_completions_impl(''','''            files);
        if (body.contains("swarmFundingV176")) {
            body_parsed["swarmFundingV176"] = body.at("swarmFundingV176");
        }
        return handle_completions_impl(''')+s[b:]
    a=s.index('    this->post_control = [this]')
    b=s.index('    this->post_responses_oai = [this]',a)
    part=s[a:b]
    part=one(part,'''        if (cmpl_id.empty()) {''','''        if (action == "swarm_funding_probe_v176") {
            res->ok({{"success",true},{"protocol","dynamic-funding-v176"}});
            return res;
        }
        if (cmpl_id.empty()) {''')
    part=one(part,'''        if (action != "reasoning_end") {''','''        if (action != "reasoning_end" && action != "swarm_funding_v176") {''')
    part=one(part,'''            task.params.control_action  = action;
            rd.post_task(std::move(task));''','''            task.params.control_action  = action;
            if (action == "swarm_funding_v176") {
                if (!swarm_funding_id_valid_v176(cmpl_id)) {
                    res->error(format_error_response("invalid funding identity", ERROR_TYPE_INVALID_REQUEST));
                    return res;
                }
                if (body.contains("authorizedOutputBillingTokens")) {
                    if (!body.at("authorizedOutputBillingTokens").is_number_unsigned()) {
                        res->error(format_error_response("invalid funding grant", ERROR_TYPE_INVALID_REQUEST));
                        return res;
                    }
                    const uint64_t tokens = body.at("authorizedOutputBillingTokens").get<uint64_t>();
                    if (tokens > std::numeric_limits<uint64_t>::max() / 4) {
                        res->error(format_error_response("funding grant overflow", ERROR_TYPE_INVALID_REQUEST));
                        return res;
                    }
                    task.params.swarm_funding_update_v176 = true;
                    task.params.swarm_funding_control_chars_v176 = swarm_funding_chars_v176(tokens);
                }
            }
            rd.post_task(std::move(task));''')
    return s[:a]+part+s[b:]

def main():
    assert platform.system()=='Darwin' and platform.machine()=='arm64', 'MAC_ARM64_REQUIRED'
    for p,expected in EXPECTED.items():
        actual=blobsha(ORIGINAL/p)
        if actual!=expected: raise RuntimeError('PINNED_RUNTIME_SOURCE_MISMATCH='+str(p)+' '+actual)
    print('PINNED_RUNTIME_SOURCE=PASS',flush=True)
    patched={SERVER/'server-task.h':patch_task((ORIGINAL/SERVER/'server-task.h').read_text()),
             SERVER/'server-context.cpp':patch_context((ORIGINAL/SERVER/'server-context.cpp').read_text())}
    root=Path(tempfile.mkdtemp(prefix='swarm-v176-runtime-',dir=HOME))
    src=root/'source'; build=root/'build'
    print('RUNTIME_STAGE='+str(root),flush=True)
    def ignore(directory,names):
        return [n for n in names if n=='.git' or (Path(directory)==ORIGINAL and (n=='build' or n.startswith('build-')))]
    shutil.copytree(ORIGINAL,src,ignore=ignore,symlinks=True)
    for p,content in patched.items(): (src/p).write_text(content)
    shutil.copy2(KIT/'swarm-funding-v176.h',src/SERVER/'swarm-funding-v176.h')
    subprocess.run(['c++','-std=c++17','-Wall','-Wextra','-Werror',str(KIT/'meter-test.cpp'),'-o',str(root/'meter-test')],check=True)
    subprocess.run([str(root/'meter-test')],check=True)
    cache=(ORIGINAL/'build-embedded-metal/CMakeCache.txt').read_text()
    flags=dict(re.findall(r'(?m)^([^#/\n][^:\n]*):(?:BOOL|STRING)=(.*)$',cache))
    opts={k:flags[k] for k in ['BUILD_SHARED_LIBS','GGML_NATIVE','GGML_ACCELERATE','GGML_METAL','GGML_METAL_EMBED_LIBRARY','GGML_BLAS','GGML_OPENMP','GGML_BACKEND_DL','LLAMA_OPENSSL','LLAMA_CURL'] if k in flags}
    opts.update(GGML_METAL='ON',GGML_METAL_EMBED_LIBRARY='ON',LLAMA_BUILD_SERVER='ON',LLAMA_BUILD_TESTS='OFF',LLAMA_BUILD_EXAMPLES='OFF',LLAMA_BUILD_COMMON='ON',CMAKE_BUILD_TYPE='Release',CMAKE_OSX_ARCHITECTURES='arm64')
    subprocess.run(['cmake','-S',str(src),'-B',str(build)]+[f'-D{k}={v}' for k,v in opts.items()],check=True)
    subprocess.run(['cmake','--build',str(build),'--config','Release','--target','llama-server','--parallel','4'],check=True)
    candidates=list((build/'bin').rglob('llama-server'))
    assert len(candidates)==1, 'RUNTIME_BINARY_NOT_UNIQUE'
    runtime=candidates[0]
    print('RUNTIME_BUILD=PASS',flush=True)
    print('RUNTIME='+str(runtime),flush=True)
    print('RUNTIME_SHA256='+hashlib.sha256(runtime.read_bytes()).hexdigest(),flush=True)
    subprocess.run([str(runtime),'--version'],check=True)
    subprocess.run(['otool','-L',str(runtime)],check=True)
    subprocess.run(['python3',str(KIT/'runtime-test.py'),str(runtime)],check=True)
    print('RUNTIME_CANDIDATE_TESTS=PASS',flush=True)
    print('INSTALLED_RUNTIME_CHANGED=NO\nDYNAMIC_FUNDING_ADVERTISEMENT=DISABLED\nPRODUCTION_CHANGES=0',flush=True)

if __name__=='__main__': main()
