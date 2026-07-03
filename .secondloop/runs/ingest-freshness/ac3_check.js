async (page) => {
  const base = 'http://127.0.0.1:8788';
  const machineId = '8d525318-baa4-4cb9-90b3-f6311ab2f3b4';

  const defResp = await page.request.get(`${base}/v1/healthz/ingest`);
  const defBody = await defResp.json();
  const defEntry = defBody.machines.find((m) => m.machine_id === machineId);

  const raisedResp = await page.request.get(`${base}/v1/healthz/ingest?stale_after_secs=14400`);
  const raisedBody = await raisedResp.json();
  const raisedEntry = raisedBody.machines.find((m) => m.machine_id === machineId);

  const badValues = ['abc', '0', '-100'];
  const badResults = [];
  for (const bad of badValues) {
    const r = await page.request.get(`${base}/v1/healthz/ingest?stale_after_secs=${bad}`);
    badResults.push({ value: bad, status: r.status(), body: await r.json().catch(() => null) });
  }

  const rows = [
    { label: 'default (7200s), machine backdated 3h', status: defResp.status(), staleFlag: defEntry.stale, expectStale: true },
    { label: 'stale_after_secs=14400, same machine', status: raisedResp.status(), staleFlag: raisedEntry.stale, expectStale: false },
  ];

  const html = `
    <html><head><style>
      body { font-family: -apple-system, monospace; background:#0b0f19; color:#e6e6e6; padding:24px; }
      h1 { color:#7dd3fc; font-size:20px; }
      table { border-collapse: collapse; width: 100%; margin-bottom: 20px; }
      td, th { border: 1px solid #334155; padding: 8px 12px; text-align: left; font-size: 13px; }
      th { color: #94a3b8; }
      .pill { display:inline-block; padding:2px 10px; border-radius:12px; font-weight:bold; }
      .ok { background:#14532d; color:#86efac; }
      .bad { background:#7f1d1d; color:#fca5a5; }
    </style></head><body>
      <h1>AC3 — stale_after_secs threshold honored + validated</h1>
      <table>
        <tr><th>Scenario</th><th>HTTP status</th><th>machine.stale</th><th>Result</th></tr>
        ${rows.map(r => `<tr><td>${r.label}</td><td>${r.status}</td><td>${r.staleFlag}</td><td><span class="pill ${r.staleFlag === r.expectStale ? 'ok' : 'bad'}">${r.staleFlag === r.expectStale ? 'PASS' : 'FAIL'}</span></td></tr>`).join('')}
      </table>
      <h1>Invalid stale_after_secs &rarr; 400</h1>
      <table>
        <tr><th>stale_after_secs value</th><th>HTTP status</th><th>Result</th></tr>
        ${badResults.map(r => `<tr><td>${r.value}</td><td>${r.status}</td><td><span class="pill ${r.status === 400 ? 'ok' : 'bad'}">${r.status === 400 ? 'PASS (400)' : 'FAIL'}</span></td></tr>`).join('')}
      </table>
    </body></html>
  `;
  await page.setContent(html);

  return { defStatus: defResp.status(), defStale: defEntry.stale, raisedStale: raisedEntry.stale, badResults };
}
