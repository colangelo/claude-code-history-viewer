/**
 * Build config for the standalone static archive webapp (`archive.html` →
 * `dist-archive/`). A sibling of the main config on purpose: the Tauri and
 * WebUI builds must keep emitting exactly today's `dist/` (tauri.conf.json
 * and the webui-server rust-embed both point at it), so the static bundle
 * gets its own output directory instead of teaching one config two modes.
 *
 * The `just archive-web-build` recipe renames the emitted `archive.html` to
 * `index.html` so any static host (or the hub's HUB_STATIC_DIR) serves it at `/`.
 */

import { defineConfig, mergeConfig } from "vite";
import path from "path";
import baseConfig from "./vite.config";

export default defineConfig(async (env) => {
  const base = await (typeof baseConfig === "function"
    ? baseConfig(env)
    : baseConfig);
  return mergeConfig(base, {
    build: {
      outDir: "dist-archive",
      emptyOutDir: true,
      rollupOptions: {
        input: path.resolve(__dirname, "archive.html"),
      },
    },
  });
});
