// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import net from 'node:net';
import {randomUUID} from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {browserFixtureUrl, commandFor, redact, root, start, validMutation} from './server.mjs';

test('write requests require exact loopback host, origin, JSON and CSRF token', () => {
  const origin = 'http://127.0.0.1:49370';
  const request = {headers: {host: '127.0.0.1:49370', origin, 'x-mewu-csrf': 'secret', 'content-type': 'application/json'}};
  assert.equal(validMutation(request, origin, 'secret'), true);
  for (const [name, value] of [['host', 'attacker.invalid:49370'], ['origin', 'http://attacker.invalid'], ['origin', undefined], ['x-mewu-csrf', 'wrong'], ['content-type', 'text/plain']]) {
    assert.equal(validMutation({headers: {...request.headers, [name]: value}}, origin, 'secret'), false);
  }
});

test('scheduler rejects shell commands, paths, empty or duplicated tests', async () => {
  for (const name of ['powershell', 'cmd', 'release', 'windows-update']) await assert.rejects(commandFor(name));
  for (const values of [[], ['../../scripts/publish-release.mjs'], ['drawing-arrow.test.mjs', 'drawing-arrow.test.mjs'], ['drawing-arrow.test.mjs; exit']]) await assert.rejects(commandFor('tests', values));
  const command = await commandFor('tests', ['drawing-arrow.test.mjs']);
  assert.deepEqual(command.args, ['--experimental-vm-modules', '--test', 'apps/desktop/src/drawing-arrow.test.mjs']);
});

test('logs filter private keys and credential forms', () => {
  const github = 'ghp_' + 'a'.repeat(36);
  const pemBegin = '-----BEGIN ' + 'PRIVATE KEY-----';
  const pemEnd = '-----END ' + 'PRIVATE KEY-----';
  const input = `Authorization: Bearer abcdef\napi_key = sensitive\npassword: other\n${github}\nhttps://person:password@localhost/path\n${pemBegin}\nnot-real-secret-material\n${pemEnd}\nnormal result`;
  const output = redact(input);
  for (const secret of ['abcdef', 'sensitive', 'other', github, 'person:password', 'not-real-secret-material']) assert.ok(!output.includes(secret));
  assert.ok(output.includes('normal result'));
});

test('browser links reject old workspaces, remote hosts and URL credentials', () => {
  const fixture = '/.private/remake-review/snapshot-ui-fixture.html';
  assert.equal(browserFixtureUrl(`http://127.0.0.1:49352${fixture}`), `http://127.0.0.1:49352${fixture}`);
  assert.ok(browserFixtureUrl(`http://127.0.0.1:49352/@fs/${root.replaceAll('\\', '/')}${fixture}`).includes('/@fs/'));
  for (const url of [`https://example.invalid${fixture}`, `http://person:credential@127.0.0.1:49352${fixture}`, `http://127.0.0.1:49352${fixture}?token=credential`, `http://127.0.0.1:49352/@fs/D:/old-repository${fixture}`, 'javascript:alert(1)']) assert.throws(() => browserFixtureUrl(url));
});

test('HTTP does not expose files or execute cross-origin mutations', async () => {
  const reservation = net.createServer();
  await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve));
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const options = {privateStateName: `test-${randomUUID()}`};
  const server = await start(port, options);
  try {
    await assert.rejects(start(port + 1, options), /控制台正在运行/);
    const base = `http://127.0.0.1:${port}`;
    const first = await (await fetch(`${base}/api/state`)).json();
    const second = await (await fetch(`${base}/api/state`)).json();
    assert.equal(first.currentSource.checkedAt, second.currentSource.checkedAt);
    const evidence = `.private/dev-check/${options.privateStateName}/browser-receipt.json`;
    await fs.writeFile(path.join(root, evidence), JSON.stringify({testOnly: true, sourceCommit: first.currentSource.head}), {flag: 'wx'});
    await promisify(execFile)(process.execPath, [path.join(root, 'scripts/dev-check/server.mjs'), 'record', '--private-state', options.privateStateName,
      '--lane', 'browser', '--status', 'passed', '--source', first.currentSource.head, '--kind', 'browser-gui', '--fingerprint', first.currentSource.fingerprint,
      '--url', 'http://127.0.0.1:49371/.private/remake-review/snapshot-ui-fixture.html', '--evidence', evidence], {cwd: root, timeout: 10000});
    const recordedResponse = await fetch(`${base}/api/state`);
    assert.equal(recordedResponse.status, 200);
    const recorded = await recordedResponse.json();
    assert.equal(recorded.lanes.browser.source, first.currentSource.head);
    assert.equal(recorded.lanes.browser.status, 'passed');
    for (const lane of ['windows', 'linux', 'mac']) assert.equal(recorded.lanes[lane].status, 'pending');
    const invalidLanes = structuredClone(recorded.lanes); invalidLanes.mac = {status: 'passed', reported: true};
    await fs.writeFile(path.join(root, `.private/dev-check/${options.privateStateName}/lanes.json`), JSON.stringify(invalidLanes));
    assert.equal((await fetch(`${base}/api/state`)).status, 400);
    await fs.writeFile(path.join(root, `.private/dev-check/${options.privateStateName}/lanes.json`), JSON.stringify(recorded.lanes));
    const page = await fetch(base);
    assert.equal(page.status, 200);
    assert.match(page.headers.get('content-security-policy'), /frame-ancestors 'none'/);
    assert.equal((await fetch(`${base}/.private/state.json`)).status, 404);
    assert.equal((await fetch(`${base}/logs/..%2Fstate.json`)).status, 404);
    const forbidden = await fetch(`${base}/api/run`, {method: 'POST', headers: {'Content-Type': 'application/json', Origin: 'https://attacker.invalid'}, body: JSON.stringify({action: 'typecheck'})});
    assert.equal(forbidden.status, 403);
    assert.equal(forbidden.headers.get('access-control-allow-origin'), null);
  } finally { await new Promise(resolve => server.close(resolve)); }
});
