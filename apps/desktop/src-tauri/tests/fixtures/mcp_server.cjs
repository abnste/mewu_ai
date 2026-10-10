// SPDX-License-Identifier: MPL-2.0
// Local protocol fixture only. No network, user files or installed app state.
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');
const { spawn } = require('node:child_process');
const [directory, mode] = process.argv.slice(2);
const children = [];
if (mode === 'tree' || mode === 'hang-init' || mode === 'slow-tree' || mode === 'parent-only-job-proof') {
  // Detached avoids libuv's private kill-on-parent-close Job in this one proof.
  // It still inherits the host Job, which grants no Windows breakaway flags.
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore', windowsHide: true, detached: mode === 'parent-only-job-proof' });
  children.push(child.pid);
}
fs.writeFileSync(path.join(directory, 'started.json'), JSON.stringify({ pid: process.pid, children }));
const send = (id, result) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id, result }) + '\n');
const fail = id => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id, error: { code: -32601, message: 'fixture unsupported method' } }) + '\n');
const schema = { type: 'object', properties: { text: { type: 'string', maxLength: 100 } }, required: ['text'], additionalProperties: false };
let lists = 0;
readline.createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  fs.appendFileSync(path.join(directory, 'requests.jsonl'), JSON.stringify(request) + '\n');
  if (mode === 'hang-init') return;
  if (request.id === undefined) return;
  switch (request.method) {
    case 'server/discover':
      if (mode === 'legacy') { fail(request.id); return; }
      send(request.id, { resultType: 'complete', supportedVersions: ['2026-07-28'], capabilities: { tools: {} }, ttlMs: 0, cacheScope: 'private' });
      break;
    case 'initialize':
      send(request.id, { protocolVersion: '2025-11-25', capabilities: { tools: {} }, serverInfo: { name: 'mewu-local-fixture', version: '1' } });
      break;
    case 'tools/list': {
      lists++;
      if (mode === 'oversized-line') { process.stdout.write('X'.repeat(1024 * 1024 + 1)); return; }
      if (mode === 'flood-stderr') { process.stderr.write('X'.repeat(4 * 1024 * 1024 + 1)); return; }
      let tools = [{ name: 'fixture.echo', description: mode === 'changed-catalog' ? 'version ' + lists : 'Echo fixture text', inputSchema: schema }];
      if (mode === 'too-many-tools') tools = Array.from({ length: 65 }, (_, i) => ({ ...tools[0], name: 'echo_' + i }));
      if (mode === 'huge-schema') tools[0].inputSchema = { ...schema, description: 'X'.repeat(16384) };
      send(request.id, { resultType: 'complete', tools, ttlMs: 0, cacheScope: 'private', ...(mode === 'loop-pages' ? { nextCursor: 'same' } : {}) });
      break;
    }
    case 'tools/call':
      if (mode === 'hang-call') return;
      if (mode === 'input-required') { send(request.id, { resultType: 'input_required', requestState: 'fixture opaque state' }); return; }
      if (mode === 'media') { send(request.id, { content: [{ type: 'image', data: 'AAAA', mimeType: 'image/png' }], isError: false }); return; }
      if (mode === 'huge-result') { send(request.id, { content: [{ type: 'text', text: 'X'.repeat(128 * 1024) }], isError: false }); return; }
      const output = {
        resultType: 'complete',
        content: [{ type: 'text', text: request.params.arguments.text }],
        isError: mode === 'tool-error',
        structuredContent: { envNames: Object.keys(process.env).sort(), text: request.params.arguments.text, pid: process.pid },
        _meta: { 'fixture-private': 'must not reach model' },
      };
      if (mode === 'slow-tree') setTimeout(() => send(request.id, output), 8000);
      else send(request.id, output);
      break;
    default: fail(request.id);
  }
});
process.stdin.on('end', () => process.exit(0));
