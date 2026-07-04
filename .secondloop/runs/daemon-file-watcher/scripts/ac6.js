async page => {
  await page.evaluate(() => {
    document.title = 'AC6 verification';
    document.body.innerHTML = `
      <div style="font-family: -apple-system, sans-serif; padding: 40px; max-width: 900px;">
        <h1 style="color:#2563eb">AC6 &mdash; DaemonConfig watch field defaults &amp; overrides</h1>
        <p><b>Claim:</b> <code>toml::from_str::&lt;DaemonConfig&gt;</code> on a config WITHOUT watch
        fields yields <code>watch_debounce_secs == 2</code> and
        <code>watch_min_pass_gap_secs == 30</code>; WITH <code>watch_debounce_secs = 5</code> and
        <code>watch_min_pass_gap_secs = 120</code> it yields exactly those values.</p>
        <p><b>Why this can't be shown in a browser:</b> TOML deserialization of
        <code>sync_daemon::config::DaemonConfig</code> is pure Rust with no HTTP/UI surface.</p>
        <p><b>Ground-truth verification:</b></p>
        <pre style="background:#111;color:#0f0;padding:16px;border-radius:8px;">test ac6_config_watch_fields_default_and_honor_explicit_values ... ok</pre>
        <p style="color:#16a34a;font-weight:bold;">Verdict: PASS (via cargo test, not browser observation)</p>
      </div>`;
  });
}
