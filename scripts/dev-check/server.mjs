// SPDX-License-Identifier: MPL-2.0
import http from 'node:http';
import fs from 'node:fs/promises';
import {watch} from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFile, spawn} from 'node:child_process';
import {promisify} from 'node:util';
import {fileURLToPath} from 'node:url';

const exec = promisify(execFile);
export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const testStateName = new URL(import.meta.url).searchParams.get('privateStateName');
if (testStateName && !/^test-[a-f0-9-]{36}$/.test(testStateName)) throw Error('Invalid isolated test state');
const privateDir = path.join(root, '.private/dev-check', testStateName || '');
const stateFile = path.join(privateDir, 'state.json');
const lanesFile = path.join(privateDir, 'lanes.json');
const lockFile = path.join(privateDir, 'server-lock.json');
const laneNames = ['browser', 'windows', 'linux', 'mac'];
const states = ['pending', 'running', 'passed', 'failed', 'blocked'];
const now = () => new Date().toISOString();
const sha = data => crypto.createHash('sha256').update(data).digest('hex');
const initial = () => ({schema: 1, jobs: [], lanes: Object.fromEntries(laneNames.map(lane => [lane, {status: 'pending', reported: true}]))});
let state = initial();
let busy = false;
let sourceCache;
let sourceVersion = 0;
let cachedVersion = -1;
let sourcePending;
let sourceWatcherFailed = false;

export function redact(text) {
  return String(text).replace(/-----BEGIN [^-]*PRIVATE KEY-----[\s\S]*?(?:-----END [^-]*PRIVATE KEY-----|$)/g, '[REDACTED PRIVATE KEY]')
    .replace(/(?:gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{16,})/g, '[REDACTED]')
    .replace(/(authorization\s*[:=]\s*(?:bearer\s+)?)[^\s"']+/gi, '$1[REDACTED]')
    .replace(/((?:api[_-]?key|password|token|secret)\s*[:=]\s*)[^\r\n,;]+/gi, '$1[REDACTED]')
    .replace(/https?:\/\/[^\s/@]+:[^\s/@]+@/gi, 'https://[REDACTED]@');
}

async function git(args) {
  return (await exec('git', ['--no-optional-locks', ...args], {cwd: root, encoding: 'utf8', timeout: 10000, maxBuffer: 16 * 1024 * 1024})).stdout;
}

export async function source(force = false) {
  if (!force && sourceWatcherFailed) throw Error('源码监控已中断，请重启控制台');
  if (!force && sourceCache && cachedVersion === sourceVersion) return sourceCache;
  if (sourcePending) { await sourcePending; if (!force) return sourceCache; }
  const version = sourceVersion;
  sourcePending = calculateSource();
  try { sourceCache = await sourcePending; cachedVersion = version; return sourceCache; }
  finally { sourcePending = undefined; }
}

async function calculateSource() {
  const [head, branch, diff, files] = await Promise.all([
    git(['rev-parse', 'HEAD']), git(['branch', '--show-current']), git(['status', '--porcelain=v1', '-z']),
    git(['ls-files', '--cached', '--others', '--exclude-standard', '-z']),
  ]);
  const hash = crypto.createHash('sha256').update(head.trim());
  for (const file of [...new Set(files.split('\0').filter(Boolean))].sort()) {
    hash.update(file).update('\0');
    try {
      const stat = await fs.lstat(path.join(root, file));
      if (stat.isSymbolicLink()) hash.update(await fs.readlink(path.join(root, file)));
      else if (stat.isFile()) hash.update(await fs.readFile(path.join(root, file)));
      else hash.update('[not-file]');
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
      hash.update('[deleted]');
    }
  }
  return {head: head.trim(), branch: branch.trim(), dirty: Boolean(diff), fingerprint: hash.digest('hex'), checkedAt: now()};
}

async function save() {
  await fs.mkdir(privateDir, {recursive: true});
  const temp = path.join(privateDir, `state-${crypto.randomUUID()}.tmp`);
  await fs.writeFile(temp, JSON.stringify(state, null, 2), {flag: 'wx'});
  await fs.rename(temp, stateFile);
}

async function loadState() {
  try {
    const data = JSON.parse(await fs.readFile(stateFile, 'utf8'));
    if (data.schema !== 1 || !Array.isArray(data.jobs) || !data.lanes) throw Error('Invalid state schema');
    state = data;
    for (const job of state.jobs) if (job.status === 'running') {
      job.status = 'interrupted'; job.finishedAt = now(); job.error = '控制台重启，未取得原进程结果';
    }
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
}

async function readLanes() {
  try {
    const lanes = JSON.parse(await fs.readFile(lanesFile, 'utf8'));
    return Object.fromEntries(laneNames.map(lane => {
      const item = lanes[lane];
      const placeholder = item?.status === 'pending' && item.reported === true && Object.keys(item).every(key => ['status', 'reported'].includes(key));
      if (item && !placeholder && (!states.includes(item.status) || !/^[a-f0-9]{40}$/.test(item.source))) throw Error('Invalid external receipt state');
      return [lane, item || {status: 'pending', reported: true}];
    }));
  } catch (error) { if (error.code === 'ENOENT') return initial().lanes; throw error; }
}

async function macHealth() {
  try {
    const bytes = await fs.readFile(path.join(privateDir, 'mac-health.json'));
    if (bytes.length > 4096) throw Error('Invalid health');
    const value = JSON.parse(bytes.toString('utf8'));
    if (value.schema !== 1 || !['ready', 'running', 'blocked', 'offline'].includes(value.status) || !['ready', 'offline', 'unknown'].includes(value.ssh) || !['ready', 'locked', 'unknown'].includes(value.desktop) || !Number.isFinite(Date.parse(value.checkedAt)) || (value.source !== null && !/^[a-f0-9]{40}$/.test(value.source))) throw Error('Invalid health');
    return {status: value.status, ssh: value.ssh, desktop: value.desktop, source: value.source, checkedAt: value.checkedAt, stale: Date.now() - Date.parse(value.checkedAt) > 120000};
  } catch { return {status: 'unknown', ssh: 'unknown', desktop: 'unknown'}; }
}

async function tests() {
  return (await fs.readdir(path.join(root, 'apps/desktop/src'))).filter(name => /^[a-z0-9-]+\.test\.mjs$/.test(name)).sort();
}

export async function commandFor(action, selected = []) {
  const app = path.join(root, 'apps/desktop');
  if (action === 'typecheck') return {cwd: app, args: [path.join(root, 'node_modules/typescript/bin/tsc'), '--noEmit']};
  if (action === 'frontend-build') return {cwd: app, args: [path.join(root, 'node_modules/vite/bin/vite.js'), 'build']};
  if (action === 'tests') {
    const allowed = await tests();
    if (!Array.isArray(selected) || !selected.length || selected.length > allowed.length || new Set(selected).size !== selected.length || selected.some(name => !allowed.includes(name))) throw Error('请选择已列出的相关测试');
    return {cwd: root, args: ['--experimental-vm-modules', '--test', ...selected.map(name => `apps/desktop/src/${name}`)]};
  }
  throw Error('Unknown action');
}

async function lockConsole() {
  await fs.mkdir(privateDir, {recursive: true});
  const ownership = {pid: process.pid, token: crypto.randomUUID()};
  try {
    const existing = JSON.parse(await fs.readFile(lockFile, 'utf8'));
    if (!Number.isInteger(existing.pid) || existing.pid < 1) throw Error('Invalid console lock');
    try { process.kill(existing.pid, 0); throw Error('已有本仓库控制台正在运行'); }
    catch (error) { if (error.code !== 'ESRCH') throw error; }
    await fs.unlink(lockFile);
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
  await fs.writeFile(lockFile, JSON.stringify(ownership), {flag: 'wx'});
  return async () => {
    try {
      const current = JSON.parse(await fs.readFile(lockFile, 'utf8'));
      if (current.token === ownership.token) await fs.unlink(lockFile);
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
  };
}

async function run(action, selected) {
  if (busy) throw Error('已有检查正在运行');
  const command = await commandFor(action, selected);
  // Reserve synchronously before any subsequent await, including source inspection.
  if (busy) throw Error('已有检查正在运行');
  busy = true;
  let job;
  try {
    job = {id: crypto.randomUUID(), action, tests: selected || [], status: 'running', kind: 'cli', source: await source(true), startedAt: now()};
    state.jobs.push(job);
    await save();
    const started = Date.now();
    const child = spawn(process.execPath, command.args, {cwd: command.cwd, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe']});
    let output = ''; let truncated = false; let timedOut = false;
    const append = chunk => { if (output.length < 4 * 1024 * 1024) output += chunk.toString('utf8'); else truncated = true; };
    child.stdout.on('data', append); child.stderr.on('data', append);
    const timer = setTimeout(() => { timedOut = true; child.kill(); }, 15 * 60 * 1000);
    child.once('error', error => { append(error.message); });
    child.once('close', async (code, signal) => {
      clearTimeout(timer);
      try {
        job.exitCode = code; job.signal = signal; job.elapsedMs = Date.now() - started;
        job.finishedAt = now(); job.status = code === 0 && !timedOut ? 'passed' : 'failed';
        job.timedOut = timedOut; job.truncated = truncated;
        job.sourceAfter = await source(true);
        job.stale = job.source.fingerprint !== job.sourceAfter.fingerprint;
        const log = redact(output);
        await fs.writeFile(path.join(privateDir, `${job.id}.log`), log, {flag: 'wx'});
        job.logSha256 = sha(log);
        await save();
      } catch { job.status = 'failed'; job.error = '保存检查结果失败'; }
      finally { busy = false; }
    });
    return job;
  } catch (error) { busy = false; throw error; }
}

export function validMutation(req, origin, token) {
  return req.headers.host === new URL(origin).host && req.headers.origin === origin && req.headers['x-mewu-csrf'] === token && req.headers['content-type'] === 'application/json';
}

export function browserFixtureUrl(input) {
  const url = new URL(input);
  const pathname = decodeURIComponent(url.pathname).replaceAll('\\', '/').toLowerCase();
  const fixture = '/.private/remake-review/snapshot-ui-fixture.html';
  if (url.protocol !== 'http:' || url.hostname !== '127.0.0.1' || Number(url.port) < 1024 || url.username || url.password || url.search || url.hash || ![fixture, `/@fs/${root.replaceAll('\\', '/').toLowerCase()}${fixture}`].includes(pathname)) throw Error('Invalid browser fixture URL');
  return url.href;
}

async function body(req) {
  let data = '';
  for await (const chunk of req) { data += chunk; if (Buffer.byteLength(data) > 32 * 1024) throw Error('Request too large'); }
  return JSON.parse(data);
}

function reply(res, code, value, type = 'application/json') {
  res.writeHead(code, {'Content-Type': `${type}; charset=utf-8`, 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff', 'X-Frame-Options': 'DENY', 'Referrer-Policy': 'no-referrer', 'Content-Security-Policy': "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"});
  res.end(type === 'application/json' ? JSON.stringify(value) : value);
}

export async function start(port = 49370, options = {}) {
  if (options.privateStateName !== undefined) {
    if (!/^test-[a-f0-9-]{36}$/.test(options.privateStateName)) throw Error('Invalid isolated test state');
    const isolatedUrl = new URL(import.meta.url);
    isolatedUrl.searchParams.set('privateStateName', options.privateStateName);
    return (await import(isolatedUrl.href)).start(port);
  }
  if (!Number.isInteger(port) || port < 1024 || port > 65535) throw Error('Invalid port');
  const unlock = await lockConsole();
  try { await loadState(); } catch (error) { await unlock(); throw error; }
  const token = crypto.randomBytes(32).toString('hex');
  const origin = `http://127.0.0.1:${port}`;
  const watchers = [];
  try {
    watchers.push(watch(root, {recursive: true}, (_event, filename) => {
      const parts = String(filename || '').replaceAll('\\', '/').split('/');
      if (parts.some(part => ['.private', 'node_modules', 'target', 'dist', 'out', 'artifacts', '.vs', '__pycache__'].includes(part))) return;
      sourceVersion++;
    }));
    const gitDir = path.resolve(root, (await git(['rev-parse', '--git-dir'])).trim());
    watchers.push(watch(gitDir, () => { sourceVersion++; }));
    sourceWatcherFailed = false;
    for (const watcher of watchers) watcher.on('error', () => { sourceVersion++; sourceWatcherFailed = true; sourceCache = undefined; });
    await source(true);
  } catch (error) { for (const watcher of watchers) watcher.close(); await unlock(); throw error; }
  const server = http.createServer(async (req, res) => {
    try {
      if (req.headers.host !== `127.0.0.1:${port}`) return reply(res, 403, {error: 'Invalid host'});
      const url = new URL(req.url, origin);
      if (req.method === 'GET' && url.pathname === '/api/state') {
        return reply(res, 200, {...state, lanes: await readLanes(), macHealth: await macHealth(), currentSource: await source(), tests: await tests(), busy, csrf: token});
      }
      if (req.method === 'POST' && url.pathname === '/api/run') {
        if (!validMutation(req, origin, token)) return reply(res, 403, {error: 'Invalid origin or token'});
        const input = await body(req);
        if (Object.keys(input).some(key => !['action', 'tests'].includes(key))) return reply(res, 400, {error: 'Unexpected input'});
        if (input.action !== 'tests' && input.tests !== undefined) return reply(res, 400, {error: 'Tests only apply to test action'});
        return reply(res, 202, await run(input.action, input.tests));
      }
      if (req.method === 'GET' && /^\/logs\/[a-f0-9-]{36}$/.test(url.pathname)) {
        const id = url.pathname.split('/')[2];
        const job = state.jobs.find(item => item.id === id);
        if (!job?.logSha256) return reply(res, 404, {error: 'Log unavailable'});
        return reply(res, 200, await fs.readFile(path.join(privateDir, `${id}.log`), 'utf8'), 'text/plain');
      }
      const assets = {'/': ['index.html', 'text/html'], '/app.js': ['app.js', 'text/javascript'], '/style.css': ['style.css', 'text/css']};
      if (req.method === 'GET' && assets[url.pathname]) {
        const [file, type] = assets[url.pathname];
        return reply(res, 200, await fs.readFile(new URL(file, import.meta.url), 'utf8'), type);
      }
      return reply(res, 404, {error: 'Not found'});
    } catch (error) { reply(res, 400, {error: redact(error.message)}); }
  });
  try { await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); }); }
  catch (error) { for (const watcher of watchers) watcher.close(); await unlock(); throw error; }
  server.once('close', () => { for (const watcher of watchers) watcher.close(); unlock().catch(() => {}); });
  console.log(origin);
  return server;
}

export async function record(args) {
  const options = {};
  for (let i = 0; i < args.length; i += 2) {
    if (!['--lane', '--status', '--source', '--evidence', '--kind', '--fingerprint', '--url'].includes(args[i]) || !args[i + 1] || options[args[i]]) throw Error('Invalid record arguments');
    options[args[i]] = args[i + 1];
  }
  const lane = options['--lane']; const status = options['--status']; const head = options['--source'];
  if (!laneNames.includes(lane) || !states.includes(status) || !/^[a-f0-9]{40}$/.test(head || '') || !['browser-gui', 'user-windows', 'windows-update', 'docker-gui', 'mac-gui', 'build'].includes(options['--kind'])) throw Error('Invalid lane/status/source/kind');
  await git(['cat-file', '-e', `${head}^{commit}`]);
  const fingerprint = options['--fingerprint'];
  if (fingerprint !== undefined && !/^[a-f0-9]{64}$/.test(fingerprint)) throw Error('Invalid fingerprint');
  let browserUrl;
  if (options['--kind'] === 'browser-gui') {
    const current = await source(true);
    if (lane !== 'browser' || fingerprint !== current.fingerprint || head !== current.head) throw Error('Browser receipt must match current source fingerprint and HEAD');
    browserUrl = browserFixtureUrl(options['--url']);
  } else if (options['--url']) throw Error('URL only allowed for browser fixture');
  const evidence = options['--evidence'];
  if (!evidence || !evidence.startsWith('.private/') || evidence.includes('\\') || evidence.split('/').includes('..') || !evidence.endsWith('.json')) throw Error('Evidence must be a private JSON receipt');
  const evidencePath = await fs.realpath(path.join(root, evidence));
  const base = await fs.realpath(path.join(root, '.private'));
  if (!evidencePath.startsWith(`${base}${path.sep}`)) throw Error('Evidence escapes private directory');
  const stat = await fs.stat(evidencePath);
  if (stat.size > 2 * 1024 * 1024) throw Error('Evidence too large');
  const bytes = await fs.readFile(evidencePath);
  JSON.parse(bytes.toString('utf8')); // Structured validation; receipt contents are never exposed by the server.
  const lanes = await readLanes();
  lanes[lane] = {status, source: head, fingerprint, browserUrl, kind: options['--kind'], reported: true, recordedAt: now(), evidenceSha256: sha(bytes), evidenceBytes: bytes.length};
  await fs.mkdir(privateDir, {recursive: true});
  const temp = path.join(privateDir, `lanes-${crypto.randomUUID()}.tmp`);
  await fs.writeFile(temp, JSON.stringify(lanes, null, 2), {flag: 'wx'});
  await fs.rename(temp, lanesFile);
  console.log(JSON.stringify({lane, status, source: head, evidenceSha256: sha(bytes)}));
}

if (!testStateName && process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv[2] === 'record') {
      const args = process.argv.slice(3);
      if (args[0] === '--private-state') {
        if (!/^test-[a-f0-9-]{36}$/.test(args[1] || '')) throw Error('Invalid isolated test state');
        const isolatedUrl = new URL(import.meta.url); isolatedUrl.searchParams.set('privateStateName', args[1]);
        await (await import(isolatedUrl.href)).record(args.slice(2));
      } else await record(args);
    }
    else if (process.argv[2] === 'serve' && process.argv.length <= 4) await start(Number(process.argv[3] || 49370));
    else throw Error('Usage: node scripts/dev-check/server.mjs serve [port] | record --lane ... --status ... --source ... --kind ... --evidence ...');
  } catch (error) { console.error(redact(error.message)); process.exitCode = 1; }
}
