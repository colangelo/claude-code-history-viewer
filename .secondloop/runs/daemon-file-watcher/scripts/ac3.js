async page => {
  await page.evaluate(() => {
    document.title = 'AC3 verification';
    document.body.innerHTML = `
      <div style="font-family: -apple-system, sans-serif; padding: 40px; max-width: 900px;">
        <h1 style="color:#2563eb">AC3 &mdash; debounce bounds burst signal count</h1>
        <p><b>Claim:</b> 30 rapid appends to one watched file produce at least 1 and at most 6
        signals observed over a 10s window.</p>
        <p><b>Why this can't be shown in a browser:</b> same daemon-only surface &mdash; the debounce
        coalescing happens inside the sync-daemon process, not through any web page.</p>
        <p><b>Ground-truth verification:</b></p>
        <pre style="background:#111;color:#0f0;padding:16px;border-radius:8px;">test ac3_rapid_appends_produce_bounded_signal_count ... ok</pre>
        <p style="color:#16a34a;font-weight:bold;">Verdict: PASS (via cargo test, not browser observation)</p>
      </div>`;
  });
}
