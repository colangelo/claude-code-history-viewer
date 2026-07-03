async (page) => {
  const base = 'http://127.0.0.1:8788';
  const machineId = 'e6717cec-838a-444c-bdcb-61108fffc2ff';
  const token = 'tok-ac1-e6717cec-838a-444c-bdcb-61108fffc2ff';

  const batch = {
    machine: { machine_id: machineId, hostname: 'host-ac1', os: 'macos' },
    projects: [{
      provider: 'claude', project_path: '/tmp/proj-ac1', name: 'proj', storage_type: 'jsonl',
      session_count: 1, message_count: 1, last_modified: null,
    }],
    sessions: [{
      provider: 'claude', session_id: 'sess-ac1', project_path: '/tmp/proj-ac1',
      file_path: '/tmp/proj-ac1/sess-ac1.jsonl', entrypoint: null, summary: 'a session',
      message_count: 1, first_message_time: null, last_message_time: null, last_modified: null,
      has_tool_use: false, has_errors: false, storage_type: 'jsonl',
    }],
    messages: [{
      provider: 'claude', session_id: 'sess-ac1', message_key: 'k1', uuid: 'u1', parent_uuid: null,
      seq: 0, timestamp: '2026-01-01T00:00:00Z', message_type: 'user', role: 'user', model: null,
      stop_reason: null, input_tokens: null, output_tokens: null, cache_creation_tokens: null,
      cache_read_tokens: null, cost_usd: null, duration_ms: null, is_sidechain: false,
      content: [{ type: 'text', text: 'hello' }],
      raw: { uuid: 'u1', text: 'hello', orig: true }, search_text: 'hello',
    }],
  };

  const ingestResp = await page.request.post(`${base}/v1/ingest`, {
    headers: { Authorization: `Bearer ${token}` },
    data: batch,
  });
  const ingestStatus = ingestResp.status();
  const ingestBody = await ingestResp.json();

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
      pre { background:#111827; padding:16px; border-radius:8px; overflow:auto; font-size:13px; }
      .field { margin: 4px 0; }
      .label { color:#94a3b8; }
    </style></head><body>
      <h1>AC1 — fresh machine reports 200 "ok" with full fields</h1>
      <div class="field"><span class="label">Seed:</span> POST /v1/ingest for machine_id=${machineId} (hostname=host-ac1) &rarr; HTTP ${ingestStatus}</div>
      <div class="field"><span class="label">Request:</span> GET ${healthUrl}</div>
      <div class="field"><span class="label">Response HTTP status:</span> <span class="pill ${healthStatus === 200 ? 'ok' : ''}">${healthStatus}</span></div>
      <div class="field"><span class="label">Response body "status":</span> <span class="pill ${healthBody.status === 'ok' ? 'ok' : ''}">${healthBody.status}</span></div>
      <div class="field"><span class="label">Matching machine entry:</span></div>
      <pre>${JSON.stringify(entry, null, 2)}</pre>
      <div class="field"><span class="label">Fields present:</span> machine_id=${!!entry.machine_id}, hostname=${!!entry.hostname}, last_seen=${!!entry.last_seen}, last_message_at=${entry.last_message_at !== undefined}, stale=${entry.stale === false}</div>
    </body></html>
  `;
  await page.setContent(html);

  return { ingestStatus, ingestBody, healthStatus, healthBodyStatus: healthBody.status, entry };
}
