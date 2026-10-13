# Swarm macOS Metal Trainer V1 — Technical Feasibility Closeout & Production Handoff

**Prepared:** 2026-10-13  
**Scope:** Apple Silicon MLX LoRA candidate, PEFT/GGUF export, llama.cpp Metal inference.  
**Decision:** Close technical-feasibility testing for this M4; do **not** install, provision, schedule, advertise, or enable training on this device. Production training integration is a separate future workstream.  
**Status (updated after 2026-10-13 backend terminal inspection):** Technical feasibility **demonstrated and closed**. The macOS v1.6.4 backend release-registry entry is private and explicitly `publicReleaseSafe:false`; no trainer release approval was found in the inspected registry references. Live training assignment and payout exclusion **remain unproven** because the due-worker policy and runtime behavior were not inspected. **NOT** production training certified, signed as a trainer, or approved for paid training.

## 1. Machine and operating constraint

- Host: Apple M4 (10 GPU/CPU topology not assumed; observed 10 CPU cores), 16 GiB unified memory; Metal 4.
- Installed production Swarm has separate GUI, supervisor, headless node, and llama.cpp 7B inference process. The production 7B model is `Qwen2.5-7B-Instruct-Q4_K_M.gguf` on `127.0.0.1:18081` as of the last Mac inspection.
- Last reported swap: 13,312.00 MiB total, 12,596.88 MiB used, ~715.12 MiB remaining. A high cumulative swap figure alone does not prove acute pressure, but it is not evidence of training headroom.
- Decision: **No larger-model training or paid trainer installation on this M4**, and no interruption of its production inference services.

## 2. Proven technical chain

`MLX native LoRA training → native MLX adapter → PEFT safetensors → GGUF LoRA adapter → isolated llama.cpp server with matching base GGUF → active LoRA (scale 1.0) → Apple M4 Metal inference`.

| Check | Observed evidence | Status |
| --- | --- | --- |
| Training attempt | `exitCode:0`, `termination:normal_exit`, 10 iterations, 12 train records; ~7.579 seconds | PASS |
| Native adapter integrity | MLX adapter SHA256 `e5130d98dcbdaecdea025c6129a5c773455d51aa70694959e58cc05b31781721` | PASS |
| PEFT export | 56 tensors, 28 modules, rank 8; base `Qwen/Qwen2.5-0.5B-Instruct` | PASS |
| GGUF conversion | 56 tensors, 1,470,496 bytes, SHA256 `d8e6fbee16c18ceec875e0319ef4fcd85aa7bf479625e8bdd65cf89fb85f3816` | PASS |
| Base model | Official Qwen2.5-0.5B-Instruct GGUF Q8_0, download SHA256 verified during test | PASS |
| Adapter loaded | `GET /lora-adapters` returned intended GGUF adapter path, id 0, `scale:1.0` | PASS |
| Apple Metal | `ggml_metal_init: found device: Apple M4`; Metal kernels compiled and loaded; inference succeeded | PASS |
| Isolated server shutdown | Port 18976 reported not listening after test | PASS |
| Production Swarm unaffected | Test used copied runtime and port 18976, no production modification commanded; full production invariant audit not performed | No known modification |

**Source evidence paths, on test Mac:**

- Test root: `/Users/m1/swarm-metal-test/results/metal-local-smoke-20261012T184807Z/`
- Training result: `result.json`, `training-local-private.log`, `mlx_config.yaml`, `initial_manifest.json`
- Native adapter: `adapter-native-mlx/adapters.safetensors`
- PEFT export: `adapter-peft/adapter_model.safetensors`, `adapter-peft/export_manifest.json`, `adapter-peft/adapter_config.json`
- GGUF: `llama-gguf-candidate/adapter-f16.gguf`, `llama-gguf-candidate/conversion_evidence.json`
- Runtime evidence: `/Users/m1/swarm-metal-test/llama-runtime-certification/{available_devices.txt,lora_adapters.json,inference_lora_metal.json,server_lora_metal.log}`
- A/B record: `/Users/m1/swarm-metal-test/llama-runtime-certification/adapter_ab_comparison.json`
- MLX evaluation: `/Users/m1/swarm-metal-test/llama-runtime-certification/mlx-evaluation/{base.log,trained.log}`
- Release review kit (not published): `/Users/m1/Downloads/Swarm_macOS_Metal_Review_Kit_v1.6.4/`

## 3. Effectiveness findings, carefully scoped

- Training-log validation loss went from 5.192 at iteration 1 to 2.271 at iteration 10.
- Paired MLX evaluation on the four **existing** validation examples: **base loss 5.192, perplexity 179.840** vs **adapter loss 2.006, perplexity 7.436**.
- This is a lower loss on a tiny validation set previously used during training. It is **not** an independent blind quality test and does **not** prove generalized real-customer improvement.
- Paired llama.cpp greedy generation: base **0/10** exact-output matches and adapter **0/10**. Responses were usually semantically sensible but verbose or misformatted; adapter showed no material exact-output improvement.
- Production performance, larger-model compatibility, and training-job economics were **not** certified.

## 4. Release and authorization contract

**Local candidate evidence observed:**

- `engine` / `candidateEngine`: `swarm-mlx-metal-lora-v1`.
- `backendRequiredEngine`: `edgeswarm-mlx-macos-metal-v1`.
- Review kit status: `BLOCKED_LOCAL_MAC_CERTIFICATION_REQUIRED` at last inspection. This candidate/backend engine-identity mismatch must be resolved at the contract/specification level before any future release; **do not casually rename either side**.
- Test result: `executionMode: local_test_only`, `backendAuthorized:false`, `hiveTrainingEnabled:false`, `productionCertified:false`, `providerRewardEligible:false`.
- The original PEFT export manifest has `llamaCppAdapterVerified:false` because it predates the later successful test; preserve this original historical record and link the subsequent compatibility evidence instead of editing old test records.

**Live backend inspection received 2026-10-13 (read-only terminal output):**

- `pm2 status swarm-ingest` reported **online** (PM2 id 4). `node --check ingest.js` emitted no syntax errors. Neither proves trainer authorization or billing controls.
- `unified_release_registry_v1.js` contains a macOS v1.6.4 private candidate with `releaseChannel: "unified_private_candidate"`, `signatureType: "adhoc"`, and `publicReleaseSafe: false`. The entry carries `validationStatus: "macos_arm64_v164_nodesmoke_3506_production"`. This indicates a node release state, **not** trainer certification.
- Registry references for earlier v1.6.2 entries include `loraTrainingAwareV1: true` alongside `loraTrainingRuntimeCertified: false`. LoRA awareness **is not** trainer authorization.
- The scoped search for the two trainer engine IDs found a match only in `enterprise_company_model_training_due_worker_v1.js`. The search did not reveal an approval in `ingest.js` or the unified release registry; this is **absence of a matching string**, not a runtime proof of exclusion.
- The backend working tree contains many pre-existing modified and untracked files (`ingest.js`, `unified_release_registry_v1.js`, payout/billing and enterprise training files among them). **Nothing in the backend was edited, restarted, committed, or pushed for this closeout.**

**Outstanding production-authorization checks — deferred to Production Training Integration:**

1. Inspect the due-worker's exact `edgeswarm-mlx-macos-metal-v1` engine eligibility logic and actual production authorization gates.
2. Verify effective node capability advertisement / task assignment prevents this `local_test_only` M4 candidate from being assigned training jobs.
3. Verify payout and settlement guards fail closed for uncertified training results; a grep search cannot prove this.
4. Validate any future signed trainer artifact and engine-ID mapping before explicitly activating a trainer release.

**Interpretation:** macOS Metal Trainer V1 **technical feasibility closes**, while production authorization/settlement certification remains intentionally **not approved and out of scope**. No claim is made that every production backend path was independently checked.

## 5. Future milestone — Production Training Integration (not part of this closeout)

Reopen only with a qualifying dedicated machine and an approved production scope. Acceptance gates:

1. Reconcile engine IDs, schema, task capability contracts, artifact identity, and manifest signing.
2. Establish hardware qualification, available-memory reservation, inference-priority/preemption policy, and safe concurrency (protect paid inference workloads).
3. Certify intended production model families (3B/7B/14B as hardware permits), correct base-model/adapter identity, data privacy, and artifact integrity.
4. Run independent holdout evaluations with a measurable, task-specific quality criterion and comparisons to the base model.
5. Validate secure training-job assignment, dataset authorization, uploads/proofs, retry/cancellation, failure handling, and exactly-once billing/provider settlement. Rejected/failed jobs earn nothing.
6. Complete packaging/signing, updater policy, staged canary, real-device recovery, and explicit release approval before public rollout.

## 6. Change-control boundary

- Do not modify `/Applications/Swarm.app`, the production headless node, the 7B inference model/runtime, or the macOS v1.6.4 release manifest for trainer work.
- Do not enable trainer task advertising, backend assignment, payout eligibility, or new capabilities based only on this 0.5B feasibility pass.
- Do not commit/push shared backend files such as `ingest.js` or release-registry files without explicit authorization and isolated diffs; unrelated in-progress changes must be preserved.
- Preserve the test artifacts and hashes as immutable evidence. When audit signoff is recorded, append a dated note rather than revising the original manifests.

## 7. Handoff verdict

**TECHNICAL_FEASIBILITY=PASS**  
**MAC_M4_TRAINER_INSTALL=NO**  
**PRODUCTION_TRAINING_CERTIFIED=NO**  
**LOCAL_CANDIDATE_PROVIDER_REWARD_ELIGIBLE=FALSE**  
**MACOS_V164_PUBLIC_RELEASE_SAFE=FALSE**  
**TRAINER_PUBLIC_RELEASE_APPROVAL=NOT_OBSERVED**  
**LIVE_TRAINER_ASSIGNMENT_AND_BILLING_FENCE=NOT_VERIFIED**  
**NEXT_OWNER=Production Training Integration milestone, future qualified hardware**