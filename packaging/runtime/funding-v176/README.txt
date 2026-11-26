Swarm v1.7.6 — Mac ARM64 runtime integration (private candidate)

Run prepare.py on the Mac with the installed Swarm node offline. It verifies two source
blob hashes from the pinned llama.cpp commit, patches a COPY of the runtime source,
builds llama-server with embedded Metal, and runs local-only diagnostics.

No installed binaries, repository working files, production configuration, customer
balances, or Git refs are modified. No capability is advertised by this kit.

Wire protocol, local runtime only:
POST /v1/chat/completions/control {"action":"swarm_funding_probe_v176"}
POST /v1/chat/completions with max_tokens:-1, stream:true, and
  "swarmFundingV176":{"id":"32 lowercase hex characters from a random UUID",
                     "authorizedOutputBillingTokens":<server authorization>}
POST /v1/chat/completions/control {"action":"swarm_funding_v176","id":"<same id>"}
  Optional authorizedOutputBillingTokens increases the confirmed grant. Grants never shrink.
  Response success:true, swarmFundingV176:{protocol,fundingPaused,pendingToken,
    pauseCount,outputChars,outputBillingChars,authorizedOutputBillingTokens,generatedTokens}.
  outputChars is monotonic raw decoded UTF-8 bytes (including a withheld sampled token).
  outputBillingChars is the projected UTF-16 size of the JSON response wrapper, including
  escapes, incomplete UTF-8 and the pending token. This is a conservative reservation
  estimate, not customer charge or provider reward authority. Final settlement still
  uses verified winning output. No fixed generation cap is added by this runtime gate.

A sampled token may wait for additional authorization, but is not resampled or emitted
until it fits. Other slots remain runnable. This first runtime path only supports single
text streams, no speculative decoder, no multimodal projector, no context shifting and
an unlimited runtime generation setting; ordinary requests remain unchanged.

The runtime tests are synthetic, with a small temporary context window and forced
continuing output. They do not change production model capacity. The meter unit test
and Python syntax were checked in the authoring environment. Full Mac C++ build,
actual scheduling behavior, the Rust worker bridge, customer-facing reload status,
backend-to-node funding, and live settlement have not been certified by this package.
