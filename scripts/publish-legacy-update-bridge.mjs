// SPDX-License-Identifier: MPL-2.0
// Root-only legacy discovery publication; never runs an installer or builds.
import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { repository } from './release-contract.mjs';
import { findRelease } from './release-api.mjs';
import { bridgePlan, maximumInstallerBytes, publishLegacyBridge, validateBridgeChanges, validateBridgeCheckout } from './legacy-update-bridge.mjs';

if (process.argv.length !== 4 || process.argv[3] !== 'ROOT-PUBLISH-LEGACY-074-BRIDGE') throw Error('Use a reviewed private plan and explicit Root legacy-bridge dispatch');
const root = fs.realpathSync(process.cwd());
const read = p => JSON.parse(fs.readFileSync(p, 'utf8'));
const git = (...args) => execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const requestPath = fs.realpathSync(process.argv[2]);
if (!requestPath.toLowerCase().startsWith((path.join(root, '.private') + path.sep).toLowerCase())) throw Error('Reviewed bridge plan must stay private');
const request = read(requestPath);
const reviewedPlan = bridgePlan(request);
const gh = (...args) => execFileSync('gh', args, { encoding: 'utf8', maxBuffer: 8 * 1024 * 1024, stdio: ['pipe', 'pipe', 'pipe'] });
function api(endpoint, body) {
  try { return JSON.parse(body === undefined ? gh('api', endpoint) : execFileSync('gh', ['api', endpoint, '--method', 'POST', '--input', '-'], { input: JSON.stringify(body), encoding: 'utf8', maxBuffer: 8 * 1024 * 1024, stdio: ['pipe', 'pipe', 'pipe'] })); }
  catch (error) { if (body === undefined && /\(HTTP 404\)/.test(String(error.stderr))) return null; throw error; }
}
function patch(endpoint, body) {
  return JSON.parse(execFileSync('gh', ['api', endpoint, '--method', 'PATCH', '--input', '-'], { input: JSON.stringify(body), encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }));
}
function tagCommit(tag) {
  let value = api(`repos/${repository}/git/ref/tags/${tag}`)?.object;
  if (!value) return null;
  for (let n = 0; value.type === 'tag' && n < 4; n++) value = api(`repos/${repository}/git/tags/${value.sha}`)?.object;
  if (value?.type !== 'commit' || !/^[a-f0-9]{40}$/.test(value.sha)) throw Error('Unsupported tag target');
  return value.sha;
}
const allowedDownloadHosts = new Set(['github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com', 'objects-origin.githubusercontent.com']);
async function download(url, expectedBytes) {
  if (!Number.isSafeInteger(expectedBytes) || expectedBytes < 1 || expectedBytes > maximumInstallerBytes) throw Error('Invalid download bound');
  let current = url;
  for (let n = 0; n <= 5; n++) {
    const parsed = new URL(current);
    if (parsed.protocol !== 'https:' || parsed.username || parsed.password || parsed.port || !allowedDownloadHosts.has(parsed.hostname)) throw Error('Untrusted bridge download origin');
    const response = await fetch(current, { headers: { 'User-Agent': 'MewuAI-legacy-update-bridge' }, redirect: 'manual', signal: AbortSignal.timeout(60000) });
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      const location = response.headers.get('location'); await response.body?.cancel();
      if (!location) throw Error('Missing download redirect');
      current = new URL(location, current).href; continue;
    }
    if (response.status !== 200 || !response.body) throw Error(`Bridge download HTTP ${response.status}`);
    const declared = response.headers.get('content-length');
    if (declared !== null && Number(declared) !== expectedBytes) { await response.body.cancel(); throw Error('Bridge download declared size differs'); }
    const chunks = []; let total = 0;
    for await (const chunk of response.body) { total += chunk.length; if (total > expectedBytes) throw Error('Bridge download exceeds exact byte bound'); chunks.push(Buffer.from(chunk)); }
    if (total !== expectedBytes) throw Error('Bridge download is incomplete');
    return Buffer.concat(chunks, total);
  }
  throw Error('Too many bridge download redirects');
}
const work = path.join(root, '.private', 'legacy-update-bridge', randomUUID());
const github = {
  release: tag => findRelease(gh, repository, tag),
  latest: () => api(`repos/${repository}/releases/latest`),
  tagCommit,
  branchCommit: branch => api(`repos/${repository}/git/ref/heads/${branch}`)?.object?.sha,
  createDraft: body => api(`repos/${repository}/releases`, body),
  upload: (_id, tag, name, bytes) => {
    fs.mkdirSync(work, { recursive: true });
    const file = path.join(work, name); fs.writeFileSync(file, bytes, { flag: 'wx' });
    if (!fs.readFileSync(file).equals(bytes)) throw Error('Private alias bytes changed');
    gh('release', 'upload', tag, '--repo', repository, file); // Deliberately no --clobber.
  },
  readBridgeAsset: (release, name) => {
    if (release.tag_name !== reviewedPlan.tag || name !== reviewedPlan.alias || release.assets.length !== 1 || release.assets[0].name !== name) throw Error('Only the reviewed bridge asset may be read');
    const directory = path.join(work, 'draft-read-' + randomUUID()); fs.mkdirSync(directory, { recursive: true });
    gh('release', 'download', reviewedPlan.tag, '--repo', repository, '--pattern', reviewedPlan.alias, '--dir', directory);
    if (fs.readdirSync(directory).join() !== reviewedPlan.alias) throw Error('Unexpected authenticated draft download');
    const file = path.join(directory, reviewedPlan.alias), stat = fs.lstatSync(file);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size !== reviewedPlan.bytes) throw Error('Authenticated draft asset size/type differs');
    return fs.readFileSync(file);
  },
  publishLatest: (id, body) => patch(`repos/${repository}/releases/${id}`, body),
};
async function assertSource(plan) {
  validateBridgeCheckout(plan, { branch: git('branch', '--show-current'), head: git('rev-parse', 'HEAD'), status: git('status', '--porcelain', '-z'),
    remoteHead: git('rev-parse', `refs/remotes/origin/${plan.workingBranch}`), protectedCommit: git('rev-parse', `refs/remotes/origin/${plan.protectedBranch}`) });
  execFileSync('git', ['merge-base', '--is-ancestor', plan.sourceCommit, plan.targetCommit], { stdio: ['ignore', 'pipe', 'pipe'] });
  const changed = execFileSync('git', ['diff', '--name-only', '--no-renames', '-z', plan.sourceCommit, plan.targetCommit], { encoding: 'utf8' }).split('\0').filter(Boolean);
  validateBridgeChanges(changed);
  if (read('apps/desktop/src-tauri/tauri.conf.json').version !== plan.sourceVersion) throw Error('Product source version changed');
  for (const file of changed) {
    const row = git('ls-tree', plan.targetCommit, '--', file);
    if (row && !row.startsWith('100644 blob ')) throw Error('Bridge release/docs changes must be regular files');
  }
}
const result = await publishLegacyBridge({ request, github, download, assertSource });
fs.mkdirSync(work, { recursive: true });
fs.writeFileSync(path.join(work, 'publication.json'), JSON.stringify(result, null, 2) + '\n', { flag: 'wx' });
console.log(JSON.stringify({ ...result, evidenceDirectory: work }));
