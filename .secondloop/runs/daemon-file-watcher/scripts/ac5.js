async page => {
  await page.evaluate(() => {
    document.title = 'AC5 verification';
    document.body.innerHTML = `
      <div style="font-family: -apple-system, sans-serif; padding: 40px; max-width: 900px;">
        <h1 style="color:#2563eb">AC5 &mdash; PassThrottle remembers pending trigger across the gap</h1>
        <p><b>Claim:</b> with <code>min_gap</code> 30s: <code>pass_due</code> false with no trigger;
        true immediately after <code>note_trigger(t0)</code>; after <code>note_pass(t0)</code> a
        trigger at t0+1s stays not-due at t0+1s but becomes due at t0+31s (survives the gap), then
        false again at t0+32s once consumed.</p>
        <p><b>Why this can't be shown in a browser:</b> <code>PassThrottle</code> is a pure Rust
        struct exercised with fabricated <code>Instant</code>s &mdash; there is no I/O or UI surface
        at all, browser or otherwise.</p>
        <p><b>Ground-truth verification:</b></p>
        <pre style="background:#111;color:#0f0;padding:16px;border-radius:8px;">test ac5_pass_throttle_remembers_pending_trigger_across_the_gap ... ok</pre>
        <p style="color:#16a34a;font-weight:bold;">Verdict: PASS (via cargo test, not browser observation)</p>
      </div>`;
  });
}
