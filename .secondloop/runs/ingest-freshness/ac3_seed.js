async (page) => {
  const base = 'http://127.0.0.1:8788';
  const machineId = '8d525318-baa4-4cb9-90b3-f6311ab2f3b4';
  const token = 'tok-ac3-8d525318-baa4-4cb9-90b3-f6311ab2f3b4';

  const batch = {
    machine: { machine_id: machineId, hostname: 'host-ac3', os: 'macos' },
    projects: [{
      provider: 'claude', project_path: '/tmp/proj-ac3', name: 'proj', storage_type: 'jsonl',
      session_count: 1, message_count: 1, last_modified: null,
    }],
    sessions: [{
      provider: 'claude', session_id: 'sess-ac3', project_path: '/tmp/proj-ac3',
      file_path: '/tmp/proj-ac3/sess-ac3.jsonl', entrypoint: null, summary: 'a session',
      message_count: 1, first_message_time: null, last_message_time: null, last_modified: null,
      has_tool_use: false, has_errors: false, storage_type: 'jsonl',
    }],
    messages: [{
      provider: 'claude', session_id: 'sess-ac3', message_key: 'k1', uuid: 'u1', parent_uuid: null,
      seq: 0, timestamp: '2026-01-01T00:00:00Z', message_type: 'user', role: 'user', model: null,
      stop_reason: null, input_tokens: null, output_tokens: null, cache_creation_tokens: null,
      cache_read_tokens: null, cost_usd: null, duration_ms: null, is_sidechain: false,
      content: [{ type: 'text', text: 'hi' }],
      raw: { uuid: 'u1', text: 'hi', orig: true }, search_text: 'hi',
    }],
  };

  const resp = await page.request.post(`${base}/v1/ingest`, {
    headers: { Authorization: `Bearer ${token}` },
    data: batch,
  });
  return { ingestStatus: resp.status() };
}
