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

console.log(`[sync-version] all files synced to ${version}.`);
