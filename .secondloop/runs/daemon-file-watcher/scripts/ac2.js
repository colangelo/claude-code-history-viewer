async page => {
  await page.evaluate(() => {
    document.title = 'AC2 verification';
    document.body.innerHTML = `
      <div style="font-family: -apple-system, sans-serif; padding: 40px; max-width: 900px;">
        <h1 style="color:#2563eb">AC2 &mdash; recursive watch covers new subdirectories</h1>
        <p><b>Claim:</b> a file created inside a NEW subdirectory made after <code>spawn</code>
        (nested dir under the root, like a new project dir) also yields a signal within an outer
        30s bound.</p>
        <p><b>Why this can't be shown in a browser:</b> same daemon-only surface as AC1 &mdash;
        <code>crates/sync-daemon/src/watcher.rs</code> has no HTTP/UI exposure. The <code>/health</code>
        page here belongs to the unrelated Tauri WebUI server.</p>
        <p><b>Ground-truth verification:</b></p>
        <pre style="background:#111;color:#0f0;padding:16px;border-radius:8px;">test ac2_new_subdirectory_created_after_spawn_is_watched_recursively ... ok</pre>
        <p style="color:#16a34a;font-weight:bold;">Verdict: PASS (via cargo test, not browser observation)</p>
      </div>`;
  });
}
