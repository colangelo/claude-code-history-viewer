#!/usr/bin/env node

/**
 * Sync the fork-owned version from package.json into the Rust workspace and
 * the Tauri config.
 *
 * Single Source of Truth: package.json (`version`).
 *
 * Targets:
 *   - Cargo.toml [workspace.package] version  (every crate inherits it via
 *     `version.workspace = true`)
 *   - src-tauri/tauri.conf.json               (webui-server / app version)
 *   - scripts/cchv-distill.py DISTILL_VERSION (the distiller is deployed as an
 *     installed COPY and announces this at every tick — #40. The line is
 *     anchored on its `# sync-version` marker; a missing marker is a hard
 *     error, never a silent no-op, because a silently stale constant is the
 *     exact failure the field exists to expose.)
 *
 * NOT a target: Cargo.lock. Refresh it with a cargo invocation
 * (`cargo check -q -p hub`) — AGENTS.md § Release Process, Guard 2.
 *
 * This is the fork's own `cchv-v*` line — NOT upstream's `v1.x` desktop
 * versions. See CLAUDE.md → Version Management.
 *
 * Usage:
 *   node scripts/sync-version.cjs   (or: just sync-version)
 */

const fs = require("fs");
const path = require("path");

const packageJsonPath = path.join(process.cwd(), "package.json");
const workspaceCargoPath = path.join(process.cwd(), "Cargo.toml");
const tauriConfPath = path.join(process.cwd(), "src-tauri", "tauri.conf.json");
const distillPath = path.join(process.cwd(), "scripts", "cchv-distill.py");

// 0. Validate every target BEFORE writing any of them, so a failure is a
//    refusal rather than a half-synced tree (Cargo.toml bumped, script not).
//    The distiller line is anchored on its marker comment so an unrelated
//    `DISTILL_VERSION` mention elsewhere in the file cannot match.
let distill = fs.readFileSync(distillPath, "utf8");
const distillRegex = /^DISTILL_VERSION = "[^"]*"  # sync-version$/m;
if (!distillRegex.test(distill)) {
  console.error(
    "[sync-version] Could not find the `DISTILL_VERSION = \"…\"  # sync-version` " +
      "marker line in scripts/cchv-distill.py. Refusing to write anything: a " +
      "distiller that announces a stale version is worse than one that announces none.",
  );
  process.exit(1);
}

// 1. Read the source of truth.
const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
const version = packageJson.version;
console.log(`[sync-version] package.json version: ${version}`);

// 2. Sync the workspace version (all crates inherit via version.workspace).
let cargoToml = fs.readFileSync(workspaceCargoPath, "utf8");
const wsRegex = /(\[workspace\.package\][^[]*?\n)version\s*=\s*"[^"]*"/;
if (!wsRegex.test(cargoToml)) {
  console.error(
    "[sync-version] Could not find [workspace.package] version in Cargo.toml.",
  );
  process.exit(1);
}
cargoToml = cargoToml.replace(wsRegex, `$1version = "${version}"`);
fs.writeFileSync(workspaceCargoPath, cargoToml);
console.log(`[sync-version] ✓ Cargo.toml [workspace.package] → ${version}`);

// 3. Sync tauri.conf.json.
const tauriConf = JSON.parse(fs.readFileSync(tauriConfPath, "utf8"));
const oldTauriVersion = tauriConf.version;
tauriConf.version = version;
fs.writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2) + "\n");
console.log(
  `[sync-version] ✓ tauri.conf.json → ${version} (was: ${oldTauriVersion})`,
);

// 4. Sync the distiller's announced version (marker validated in step 0, so
//    this cannot be the step that fails after the others have written).
distill = distill.replace(
  distillRegex,
  `DISTILL_VERSION = "${version}"  # sync-version`,
);
fs.writeFileSync(distillPath, distill);
console.log(`[sync-version] ✓ scripts/cchv-distill.py DISTILL_VERSION → ${version}`);

console.log(`[sync-version] all files synced to ${version}.`);
