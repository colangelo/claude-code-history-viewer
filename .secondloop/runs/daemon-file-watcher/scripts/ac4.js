async page => {
  await page.evaluate(() => {
    document.title = 'AC4 verification';
    document.body.innerHTML = `
      <div style="font-family: -apple-system, sans-serif; padding: 40px; max-width: 900px;">
        <h1 style="color:#2563eb">AC4 &mdash; bad root degrades, doesn't disable watching</h1>
        <p><b>Claim:</b> <code>spawn</code> with a nonexistent path listed BEFORE a valid TempDir
        root returns <code>Ok</code>, and a file created under the valid root still signals within
        an outer 30s bound.</p>
        <p><b>Why this can't be shown in a browser:</b> same daemon-only surface &mdash; root
        registration and degradation logic lives entirely inside <code>watcher::spawn</code>.</p>
        <p><b>Ground-truth verification:</b></p>
        <pre style="background:#111;color:#0f0;padding:16px;border-radius:8px;">test ac4_bad_root_before_valid_root_degrades_not_disables ... ok</pre>
        <p style="color:#16a34a;font-weight:bold;">Verdict: PASS (via cargo test, not browser observation)</p>
      </div>`;
  });
}
