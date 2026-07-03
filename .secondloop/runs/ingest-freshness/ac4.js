async (page) => {
  const base = 'http://127.0.0.1:8788';
  const machineId = '14a6cc6a-75c4-4a8b-91f7-4ca75a5d142d';
  const token = 'tok-ac4-14a6cc6a-75c4-4a8b-91f7-4ca75a5d142d';

  const batch = {
    machine: { machine_id: machineId, hostname: 'host-ac4', os: 'macos' },
    projects: [{
      provider: 'claude', project_path: '/tmp/proj-ac4', name: 'proj', storage_type: 'jsonl',
      session_count: 1, message_count: 0, last_modified: null,
    }],
    sessions: [{
      provider: 'claude', session_id: 'sess-ac4', project_path: '/tmp/proj-ac4',
      file_path: '/tmp/proj-ac4/sess-ac4.jsonl', entrypoint: null, summary: 'a session',
      message_count: 0, first_message_time: null, last_message_time: null, last_modified: null,
      has_tool_use: false, has_errors: false, storage_type: 'jsonl',
    }],
    messages: [],
  };

  const ingestResp = await page.request.post(`${base}/v1/ingest`, {
    headers: { Authorization: `Bearer ${token}` },
    data: batch,
  });
  const ingestStatus = ingestResp.status();

  const healthUrl = `${base}/v1/healthz/ingest?stale_after_secs=999999999`;
  const healthResp = await page.request.get(healthUrl);
  const healthStatus = healthResp.status();
  const healthBody = await healthResp.json();
  const entry = healthBody.machines.find((m) => m.machine_id === machineId);

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
    </style></head><body>
      <h1>AC4 — zero messages: last_message_at null, stale ignores message recency</h1>
      <div class="field"><span class="label">Seed:</span> POST /v1/ingest with messages=[] for machine_id=${machineId} &rarr; HTTP ${ingestStatus} (0 messages persisted)</div>
      <div class="field"><span class="label">Request:</span> GET ${healthUrl}</div>
      <div class="field"><span class="label">Response HTTP status:</span> <span class="pill ${healthStatus === 200 ? 'ok' : 'bad'}">${healthStatus}</span></div>
      <div class="field"><span class="label">Response body "status":</span> <span class="pill ${healthBody.status === 'ok' ? 'ok' : 'bad'}">${healthBody.status}</span></div>
      <div class="field"><span class="label">Matching machine entry:</span></div>
      <pre>${JSON.stringify(entry, null, 2)}</pre>
      <div class="field">last_message_at === null: <span class="pill ${entry.last_message_at === null ? 'ok' : 'bad'}">${entry.last_message_at === null}</span></div>
      <div class="field">stale === false: <span class="pill ${entry.stale === false ? 'ok' : 'bad'}">${entry.stale}</span></div>
    </body></html>
  `;
  await page.setContent(html);

  return { ingestStatus, healthStatus, bodyStatus: healthBody.status, entry };
}
