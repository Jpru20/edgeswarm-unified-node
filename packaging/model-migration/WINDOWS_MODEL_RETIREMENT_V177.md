# Windows v1.7.7 model retirement

Retirement is opt-in, signed, and Windows-only. A model is not retired because
it loses primary priority. Existing nodes that do not understand the optional
field continue normal operation. No production retirement is authorized by this
document.

## Signed registry publisher

Add the optional `windowsModelRetirementsV177` array inside the existing signed
registry payload (before base64 encoding and EIP-191 signing). Omission or an
empty array means no retirement. Do not put it outside the signed payload.

Each entry has:
- `retiredModel`: exact existing selectedModel.
- `replacementModel`: exact existing selectedModel, same capability/tier/runtime,
  higher primary priority (lower numerical priority), not experimental.
- `artifactSha256`: the exact single base GGUF SHA from the retiring definition.
- `deprecatedAtUnix`: explicit policy epoch in Unix seconds, nonzero and no later
  than registry issuedAtUnix.

Keep both definitions in the registry while the directive is present, including
their existing hashes and execution settings. The old definition must have a
single non-shared model artifact and no vision capability. Do not list a
replacement as retired. Publishing an empty array cancels pending retirements.
Use a new deprecation epoch if reinstating a previously canceled retirement.

## Node behavior

* The existing recommendation and capacity-certification flow installs the
  replacement; the existing priority migration chooses the certified model.
* At a successful idle registry refresh, an unchanged healthy replacement
  becomes eligible for the retirement grace observation. The node verifies
  both models are locally certified and the active runtime's actual model path.
* Seven full days must elapse after the first valid local observation under the
  signed rule. Returning to a different active model, changing the rule or
  artifact, clock rollback, or a gap exceeding 24 hours resets the observation.
  The node never starts cleanup using an expired or cache-only policy.
* The node reuses the existing authenticated idle withdrawal endpoint and
  device-lock RPC. For cleanup, fromModel is the unchanged active replacement
  and toModel is the certified standby. The RPC only withdraws assignment
  eligibility; it never changes the active runtime. This prevents deletion
  with prepared, leased, quiesced, finalizing or legacy assigned work.
* With the permit, recheck the exact active and standby SHA-256s, ownership,
  plain file paths, expiry, and stop state. Remove only the verified inactive
  GGUF and its own receipt. Refresh the certified roster before readmission.
* Subsequent startup provisioning skips a retired artifact only if its
  replacement remains certified locally. A revoked policy or unavailable
  replacement permits normal provisioning again.
* Never remove a user/custom model directory, a symlink/reparse point,
  a vision/projector pack, shared model artifact, or company/training data.
  No recursive deletion or certificate deletion is performed.

## Existing installations

New downloads to the standard Swarm-owned Windows model folder get a
`.swarm-owned-v177.json` receipt after SHA-verified installation. A previously
existing file does not get a receipt merely because it matches a model hash.
Older files without reliable ownership provenance are retained until a separate
provider-approved adoption flow is implemented. No automatic legacy-file
adoption or deletion is included.

## Backend activation

Before publishing a Windows updater manifest, register the exact new Windows
v1.7.7 headless binary and desktop hashes using the existing release allowlist.
Authorize the matching Windows release identity on the existing
`/node/model-migration/begin` endpoint; retain the deployed macOS allowance.
No new SQL migration or separate withdrawal authority is required. The
backend should also avoid selecting a deprecated standby on a device that has
its preferred replacement, otherwise standby use restarts the grace period.
Do not publish a nonempty retirement directive until the Windows candidate and
backend policy are verified. Keep Mac and other existing release artifacts
untouched.
