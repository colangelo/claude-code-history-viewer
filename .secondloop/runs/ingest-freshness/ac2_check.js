async (page) => {
  const base = 'http://127.0.0.1:8788';
  const freshId = '53a7e68d-4f4f-4174-a35f-e8ae083130f1';
  const staleId = '7b993ba6-af27-466a-9698-33b57a78ec97';

  const healthUrl = `${base}/v1/healthz/ingest`;
  const resp = await page.request.get(healthUrl);
  const status = resp.status();
  const body = await resp.json();
  const fresh = body.machines.find((m) => m.machine_id === freshId);
  const stale = body.machines.find((m) => m.machine_id === staleId);

  const html = `
    <html><head><style>
      body { font-family: -apple-system, monospace; background:#0b0f19; color:#e6e6e6; padding:24px; }
      h1 { color:#7dd3fc; font-size:20px; }
      .pill { display:inline-block; padding:2px 10px; border-radius:12px; font-weight:bold; }
      .ok { background:#14532d; color:#86efac; }
      .bad { background:#7f1d1d; color:#fca5a5; }
      pre { background:#111827; padding:16px; border-radius:8px; overflow:auto; font-size:13px; }
      .field { margin: 4px 0; }
      .label { color:#94a3b8; }
      .cols { display:flex; gap:16px; }
      .col { flex:1; }
    </style></head><body>
      <h1>AC2 — one stale machine (backdated 3h) triggers 503, entries stay accurate</h1>
      <div class="field"><span class="label">Request:</span> GET ${healthUrl} (default stale_after_secs=7200)</div>
      <div class="field"><span class="label">Response HTTP status:</span> <span class="pill ${status === 503 ? 'ok' : 'bad'}">${status}</span></div>
      <div class="field"><span class="label">Response body "status":</span> <span class="pill ${body.status === 'stale' ? 'ok' : 'bad'}">${body.status}</span></div>
      <div class="cols">
        <div class="col">
          <div class="field"><span class="label">Fresh machine (last_seen just now):</span></div>
          <pre>${JSON.stringify(fresh, null, 2)}</pre>
          <div class="field">stale === false: <span class="pill ${fresh.stale === false ? 'ok' : 'bad'}">${fresh.stale}</span></div>
        </div>
        <div class="col">
          <div class="field"><span class="label">Backdated machine (last_seen -3h):</span></div>
          <pre>${JSON.stringify(stale, null, 2)}</pre>
          <div class="field">stale === true: <span class="pill ${stale.stale === true ? 'ok' : 'bad'}">${stale.stale}</span></div>
        </div>
      </div>
    </body></html>
  `;
  await page.setContent(html);

  return { status, bodyStatus: body.status, fresh, stale };
}
