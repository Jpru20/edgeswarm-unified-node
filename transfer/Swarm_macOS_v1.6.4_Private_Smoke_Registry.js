#!/usr/bin/env node
"use strict";

// Adds exactly one private, exact-hash macOS v1.6.4 smoke-test trust record.
// Run only from the production ~/swarm-data checkout.
// Intentionally does not alter the public desktop updater or compliance gates.

const fs = require("fs");
const path = require("path");
const cp = require("child_process");
const assert = require("assert");

const file = path.resolve(process.cwd(), "unified_release_registry_v1.js");
const matcher = "getApprovedUnifiedReleaseV1";
const candidate = {
  platform: "macos",
  version: "1.6.4",
  architecture: "arm64",
  packageType: "unified_binary",
  releaseChannel: "unified_private_candidate",
  unifiedProtocolVersion: "edgeswarm-unified-heartbeat-v1",
  providerExeSha256: "33c1d7f4962d15260385105a1f7448c688605799ca9be721ce4165a16a61d666",
  runtimeSha256: "33c1d7f4962d15260385105a1f7448c688605799ca9be721ce4165a16a61d666",
  desktopExeSha256: "b9540a70b130a83dda47ffee4502e7469a0a4f50e7883bce875841c598505ed3",
  sourceCommit: "b34e4a0bec087b09b509198a3a6bd333dbeba178",
  buildMethod: "guarded_macos_release_v1",
  packagingMethod: "native_hdiutil_v1",
  unsignedDmgArtifactSha256: "7db8f7f1e84248abe540d1732009bdc08950c363a8ffbd4eb2ee0b16e7b65cd0",
  updaterArchiveSha256: "ab30973af36241f30f09bd69e753bd2245196b6d56ea1f521d4e080a2d0f1469",
  updaterArchiveSignatureSha256: "5334e9a1d43bb5a913b97b15efd6110bdec7efce39965ba09a2b9fe7ebc32e33",
  releaseCandidate: true,
  privateCandidate: true,
  releaseLabel: "v1.6.4-macos-private-smoke",
  compiledProductionConfigVerified: true,
  appBundlePayloadVerified: true,
  updaterPayloadVerified: true,
  authRestoreCertified: true,
  updaterCertificationPassed: false,
  exactPreviousVersionTransitionCertified: false,
  signatureType: "adhoc",
  notarized: false,
  paidAssignmentSafe: true,
  publicReleaseSafe: false,
  validationStatus: "macos_arm64_v164_private_smoke_pending_nodesmoke_weekly_compliance"
};

function load() {
  delete require.cache[require.resolve(file)];
  const registry = require(file);
  assert(typeof registry[matcher] === "function", "release matcher missing");
  assert(Array.isArray(registry.UNIFIED_APPROVED_RELEASES_V1), "release entries missing");
  return registry;
}
function verify(registry) {
  const matching = registry.UNIFIED_APPROVED_RELEASES_V1.filter(r =>
    r.platform === candidate.platform &&
    r.version === candidate.version &&
    r.architecture === candidate.architecture
  );
  assert(matching.length === 1, "expected exactly one macOS v1.6.4 arm64 row");
  const match = matching[0];
  assert.strictEqual(match.providerExeSha256 || match.runtimeSha256, candidate.providerExeSha256);
  assert.strictEqual(match.publicReleaseSafe, false);
  assert.strictEqual(match.paidAssignmentSafe, true);
  const input = {
    platform: candidate.platform, appVersion: candidate.version,
    architecture: candidate.architecture, packageType: candidate.packageType,
    releaseChannel: candidate.releaseChannel,
    unifiedProtocolVersion: candidate.unifiedProtocolVersion,
    providerExeSha256: candidate.providerExeSha256
  };
  const accepted = registry[matcher](input);
  assert(accepted, "exact smoke executable not recognized");
  assert.strictEqual(accepted.runtimeSha256, candidate.runtimeSha256);
  const wrong = Object.assign({}, input, {providerExeSha256: "0".repeat(64)});
  assert(!registry[matcher](wrong), "wrong hash must fail closed");
  console.log("MACOS_V164_PRIVATE_SMOKE_MATCH=PASS");
  console.log("WRONG_RUNTIME_HASH_REJECTED=PASS");
  console.log("PUBLIC_RELEASE_SAFE=false");
  console.log("PAID_SMOKE_ASSIGNMENT_ALLOWED=true");
  console.log("WEEKLY_COMPLIANCE=UNCHANGED");
  console.log("PUBLIC_UPDATER_MANIFEST=UNCHANGED");
}

assert(fs.existsSync(file), "run from ~/swarm-data with unified_release_registry_v1.js");
assert(!fs.lstatSync(file).isSymbolicLink(), "registry must not be symlink");
const original = fs.readFileSync(file, "utf8");
const before = load();
const existing = before.UNIFIED_APPROVED_RELEASES_V1.filter(r =>
  r.platform === "macos" && r.version === "1.6.4" && r.architecture === "arm64"
);
if (existing.length !== 0) {
  verify(before);
  console.log("MANIFEST_UPDATE=ALREADY_PRESENT");
  process.exit(0);
}

const anchor = "const UNIFIED_APPROVED_RELEASES_V1 = Object.freeze([";
assert(original.split(anchor).length === 2, "release array anchor changed; no edit");
const rendered = "    // MACOS_V164_PRIVATE_SMOKE_EXACT_HASH_V1\n    " +
  JSON.stringify(candidate, null, 4).replace(/\n/g, "\n    ") + ",\n";
const updated = original.replace(anchor, anchor + "\n\n" + rendered);
const temp = file + ".macos-v164-smoke." + process.pid + ".tmp.js";
const backupRoot = path.join(require("os").homedir(), ".swarm-backups");
fs.mkdirSync(backupRoot, {recursive: true, mode: 0o700});
const backup = path.join(backupRoot, "unified_release_registry_v1.pre-macos-v164-smoke." + new Date().toISOString().replace(/[:.]/g, "-") + ".bak");
const perms = fs.statSync(file).mode & 0o777;

try {
  fs.writeFileSync(temp, updated, {mode: perms, flag: "wx"});
  const checked = cp.spawnSync(process.execPath, ["--check", temp], {encoding: "utf8"});
  assert(checked.status === 0, "updated registry syntax invalid: " + (checked.stderr || ""));
  fs.copyFileSync(file, backup, fs.constants.COPYFILE_EXCL);
  fs.renameSync(temp, file);
  try {
    const after = load();
    assert(after.UNIFIED_APPROVED_RELEASES_V1.length ===
      before.UNIFIED_APPROVED_RELEASES_V1.length + 1, "unrelated entry count changed");
    for (let i = 0; i < before.UNIFIED_APPROVED_RELEASES_V1.length; i++) {
      assert.deepStrictEqual(after.UNIFIED_APPROVED_RELEASES_V1[i + 1],
        before.UNIFIED_APPROVED_RELEASES_V1[i], "preexisting registry entry altered");
    }
    verify(after);
    console.log("MANIFEST_UPDATE=ADDED");
    console.log("MANIFEST_BACKUP=" + backup);
  } catch (error) {
    fs.copyFileSync(backup, file);
    throw error;
  }
} finally {
  if (fs.existsSync(temp)) fs.unlinkSync(temp);
}
