async (page) => {
  const base = 'http://127.0.0.1:8788';
  const machineId = 'fba1f566-3db4-4bda-8a11-b042f8f9b8bf';
  const token = 'tok-ac5-fba1f566-3db4-4bda-8a11-b042f8f9b8bf';

  const batch = {
    machine: { machine_id: machineId, hostname: 'host-ac5', os: 'macos' },
    projects: [], sessions: [], messages: [],
  };
  await page.request.post(`${base}/v1/ingest`, {
    headers: { Authorization: `Bearer ${token}` },
    data: batch,
  });

  // No Authorization header at all on this request.
  const healthzResp = await page.request.get(`${base}/v1/healthz`);
  const ingestHealthResp = await page.request.get(`${base}/v1/healthz/ingest`);
  const ingestHealthBody = await ingestHealthResp.json();

  // Contrast: the bearer-authed ingest endpoint DOES reject a missing token.
  const unauthedIngestResp = await page.request.post(`${base}/v1/ingest`, {
    data: batch,
  });

  const html = `
    <html><head><style>
      body { font-family: -apple-system, monospace; background:#0b0f19; color:#e6e6e6; padding:24px; }
      h1 { color:#7dd3fc; font-size:20px; }
      table { border-collapse: collapse; width: 100%; }
      td, th { border: 1px solid #334155; padding: 8px 12px; text-align: left; font-size: 13px; }
      th { color: #94a3b8; }
      .pill { display:inline-block; padding:2px 10px; border-radius:12px; font-weight:bold; }
      .ok { background:#14532d; color:#86efac; }
      .bad { background:#7f1d1d; color:#fca5a5; }
    </style></head><body>
      <h1>AC5 — /v1/healthz/ingest answers with no Authorization header (matches /v1/healthz policy)</h1>
      <table>
        <tr><th>Request (no Authorization header)</th><th>HTTP status</th><th>Result</th></tr>
        <tr><td>GET /v1/healthz</td><td>${healthzResp.status()}</td><td><span class="pill ok">reference: unauthenticated by design</span></td></tr>
        <tr><td>GET /v1/healthz/ingest</td><td>${ingestHealthResp.status()}</td><td><span class="pill ${[200,503].includes(ingestHealthResp.status()) ? 'ok' : 'bad'}">${[200,503].includes(ingestHealthResp.status()) ? 'PASS (not 401/403)' : 'FAIL'}</span></td></tr>
        <tr><td>POST /v1/ingest (contrast: bearer-authed route)</td><td>${unauthedIngestResp.status()}</td><td><span class="pill ${unauthedIngestResp.status() === 401 ? 'ok' : 'bad'}">${unauthedIngestResp.status() === 401 ? 'expected 401 (proves this run truly omits auth)' : 'unexpected'}</span></td></tr>
      </table>
      <div style="margin-top:16px">/v1/healthz/ingest body: <pre style="background:#111827;padding:16px;border-radius:8px;">${JSON.stringify(ingestHealthBody, null, 2).slice(0, 400)}...</pre></div>
    </body></html>
  `;
  await page.setContent(html);

  return {
    healthzStatus: healthzResp.status(),
    ingestHealthStatus: ingestHealthResp.status(),
    unauthedIngestStatus: unauthedIngestResp.status(),
  };
}
