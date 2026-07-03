async (page) => {
  const base = 'http://127.0.0.1:8788';
  const freshId = '53a7e68d-4f4f-4174-a35f-e8ae083130f1';
  const staleId = '7b993ba6-af27-466a-9698-33b57a78ec97';
  const freshToken = 'tok-ac2fresh-53a7e68d-4f4f-4174-a35f-e8ae083130f1';
  const staleToken = 'tok-ac2stale-7b993ba6-af27-466a-9698-33b57a78ec97';

  function batchFor(machineId, hostname, session) {
    return {
      machine: { machine_id: machineId, hostname, os: 'macos' },
      projects: [{
        provider: 'claude', project_path: `/tmp/${session}`, name: 'proj', storage_type: 'jsonl',
        session_count: 1, message_count: 1, last_modified: null,
      }],
      sessions: [{
        provider: 'claude', session_id: session, project_path: `/tmp/${session}`,
        file_path: `/tmp/${session}/${session}.jsonl`, entrypoint: null, summary: 'a session',
        message_count: 1, first_message_time: null, last_message_time: null, last_modified: null,
        has_tool_use: false, has_errors: false, storage_type: 'jsonl',
      }],
      messages: [{
        provider: 'claude', session_id: session, message_key: 'k1', uuid: 'u1', parent_uuid: null,
        seq: 0, timestamp: '2026-01-01T00:00:00Z', message_type: 'user', role: 'user', model: null,
        stop_reason: null, input_tokens: null, output_tokens: null, cache_creation_tokens: null,
        cache_read_tokens: null, cost_usd: null, duration_ms: null, is_sidechain: false,
        content: [{ type: 'text', text: 'hi' }],
        raw: { uuid: 'u1', text: 'hi', orig: true }, search_text: 'hi',
      }],
    };
  }

  const freshResp = await page.request.post(`${base}/v1/ingest`, {
    headers: { Authorization: `Bearer ${freshToken}` },
    data: batchFor(freshId, 'host-ac2-fresh', 'sess-ac2-fresh'),
  });
  const staleResp = await page.request.post(`${base}/v1/ingest`, {
    headers: { Authorization: `Bearer ${staleToken}` },
    data: batchFor(staleId, 'host-ac2-stale', 'sess-ac2-stale'),
  });

  return { freshStatus: freshResp.status(), staleStatus: staleResp.status() };
}
