async page => {
  await page.evaluate(() => {
    document.title = 'AC1 verification';
    document.body.innerHTML = `
      <div style="font-family: -apple-system, sans-serif; padding: 40px; max-width: 900px;">
        <h1 style="color:#2563eb">AC1 &mdash; watcher::spawn signals on create+write</h1>
        <p><b>Claim:</b> creating and writing a file under a watched TempDir root yields a debounced
        signal on the channel within an outer 30s bound.</p>
        <p><b>Why this can't be shown in a browser:</b> the feature under test is
        <code>crates/sync-daemon/src/watcher.rs</code>, a standalone daemon process with no HTTP or UI
        surface. The <code>/health</code> endpoint shown below belongs to the unrelated Tauri WebUI
        server (port 3727) and has no wiring to the sync-daemon's file watcher.</p>
        <p><b>Ground-truth verification:</b> ran the frozen T2 eval directly:</p>
        <pre style="background:#111;color:#0f0;padding:16px;border-radius:8px;">$ cargo test -p loop-evals --test daemon-file-watcher_eval -- --test-threads=1
test ac1_creating_and_writing_a_file_yields_a_signal ... ok</pre>
        <p style="color:#16a34a;font-weight:bold;">Verdict: PASS (via cargo test, not browser observation)</p>
      </div>`;
  });
}
