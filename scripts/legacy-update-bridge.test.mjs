// SPDX-License-Identifier: MPL-2.0
// Dummy bytes and a fully offline GitHub transport. Never creates public data.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { releasePlan, updateUrl } from './release-contract.mjs';
import { bridgePlan, canonicalAssetUrl, compareLegacyVersions, legacyVersion, publishLegacyBridge, validateBridgeChanges } from './legacy-update-bridge.mjs';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const clone = value => structuredClone(value);
function fixture(version = '1.0.0-preview.4') {
  const bytes = Buffer.from('owned fake installer bytes; never executable');
  const request = { schemaVersion: 1, source: { version, commit: 'a'.repeat(40), installer: { bytes: bytes.length, sha256: hash(bytes) } },
    targetCommit: 'b'.repeat(40), expectedMasterCommit: 'c'.repeat(40), expectedLatest: { id: 7, tag: 'v0.7.4', commit: 'd'.repeat(40) } };
  const plan = bridgePlan(request), calls = [], downloads = new Map(); let assetId = 10;
  const asset = (tag, name, content) => { downloads.set(canonicalAssetUrl(tag, name), Buffer.from(content)); return { id: assetId++, name, size: content.length, digest: 'sha256:' + hash(content), browser_download_url: canonicalAssetUrl(tag, name) }; };
  const signature = Buffer.from(`untrusted comment: fixture\nfixture\ntrusted comment: version:${version}\nfixture\n`).toString('base64');
  const manifest = { version, platforms: Object.fromEntries(['windows-x86_64', 'darwin-aarch64', 'linux-x86_64'].map(p => [p, { url: updateUrl(version, p), signature }])) };
  const state = {
    source: { id: 8, tag_name: plan.sourceTag, target_commitish: plan.sourceCommit, draft: false, prerelease: true,
      assets: Object.values(releasePlan(version)).flatMap(p => p.files).map(name => asset(plan.sourceTag, name, name === plan.sourceName ? bytes : Buffer.from(name))) },
    channel: { id: 9, tag_name: 'update-channel', target_commitish: 'e'.repeat(40), draft: false, prerelease: true,
      assets: [asset('update-channel', 'preview.json', Buffer.from(JSON.stringify(manifest))), asset('update-channel', releasePlan(version)['darwin-aarch64'].updater, Buffer.from('fake updater tar'))] },
    latest: { id: 7, tag_name: 'v0.7.4', target_commitish: 'master', draft: false, prerelease: false, assets: [] },
    bridge: null, master: request.expectedMasterCommit, refs: { [plan.sourceTag]: plan.sourceCommit, 'v0.7.4': request.expectedLatest.commit },
  };
  function completeBridge(draft) {
    state.bridge = { id: 100, tag_name: plan.tag, target_commitish: plan.targetCommit, name: plan.title, body: plan.body, draft, prerelease: false,
      assets: [asset(plan.tag, plan.alias, bytes)] };
    if (!draft) state.refs[plan.tag] = plan.targetCommit;
  }
  const github = {
    release: async tag => clone(tag === plan.sourceTag ? state.source : tag === 'update-channel' ? state.channel : state.bridge),
    latest: async () => clone(state.latest),
    branchCommit: async () => state.master,
    tagCommit: async tag => state.refs[tag] ?? null,
    createDraft: async body => { calls.push({ action: 'draft', body: clone(body) }); state.bridge = { id: 100, ...clone(body), assets: [] }; return clone(state.bridge); },
    upload: async (id, tag, name, content) => {
      calls.push({ action: 'upload', id, tag, name, sha256: hash(content) }); assert.deepEqual(Buffer.from(content), bytes);
      state.bridge.assets.push(asset(tag, name, content)); await state.afterUpload?.();
    },
    readBridgeAsset: async (release, name) => { calls.push({ action: 'authenticated-draft-read', id: release.id, name }); return Buffer.from(downloads.get(plan.aliasUrl)); },
    publishLatest: async (id, body) => {
      calls.push({ action: 'publish-latest', id, body: clone(body) }); assert.equal(state.bridge.assets.length, 1);
      assert.ok(calls.some(c => c.action === 'authenticated-draft-read') || !state.bridge.draft);
      state.bridge.draft = false; state.latest = clone(state.bridge); state.refs[plan.tag] = plan.targetCommit; await state.afterPublish?.();
    },
  };
  const download = async (url, bound) => {
    calls.push({ action: 'anonymous-read', url });
    if (url === plan.aliasUrl && state.bridge?.draft) throw Error('HTTP404: anonymous draft asset is unavailable');
    const value = downloads.get(url); assert.ok(value, `Unexpected URL ${url}`); assert.equal(value.length, bound); return Buffer.from(value);
  };
  const run = () => publishLegacyBridge({ request, github, download, assertSource: async p => { assert.equal(p.sourceCommit, request.source.commit); state.assertSourceCalls = (state.assertSourceCalls ?? 0) + 1; if (state.invalidSource) throw Error('Runtime changed'); } });
  return { request, plan, bytes, calls, state, github, downloads, asset, completeBridge, run };
}
const mutations = f => f.calls.filter(c => ['draft', 'upload', 'publish-latest'].includes(c.action));

test('0.7.4 numeric four-field bridge selects exact NSIS alias, canonical URL and /S', () => {
  const { plan } = fixture(); assert.equal(plan.tag, 'v1.0.0.4'); assert.equal(plan.alias, 'MewuAI-Remake-Setup-1.0.0.4-win-x64.exe');
  assert.equal(plan.aliasUrl, canonicalAssetUrl(plan.tag, plan.alias)); assert.equal(plan.installerKind, 'RemakeNsis'); assert.deepEqual(plan.installerArguments, ['/S']);
  assert.match(plan.body, /实际安装的软件仍是 \*\*1\.0\.0-preview\.4\*\*/); assert.match(plan.body, new RegExp(plan.sourceCommit));
  assert.ok(compareLegacyVersions(plan.tag, 'v0.7.4.0') > 0); assert.deepEqual(legacyVersion('v0.7.4'), [0, 7, 4, -1]);
});
test('reject malformed/overflow versions, downgrade and genuine stable replacement', () => {
  for (const tag of ['v1.0.0-preview.4', '1.0.0.4', 'v1.0', 'v1.0.0.4.1', 'v1.0.0.-1', 'v2147483648.0.0']) assert.throws(() => legacyVersion(tag));
  for (const latest of ['v1.0.0.4', 'v1.0.0.5', 'v1.0.0']) { const f = fixture(); f.request.expectedLatest.tag = latest; assert.throws(() => bridgePlan(f.request), /downgrade|repeat|stable/); }
});
test('only exact public release/docs files admit a new target; runtime, agent files and arbitrary scripts do not', () => {
  assert.equal(validateBridgeChanges(['README.md', 'apps/desktop/README.md', 'docs/legacy-updates.md', 'docs/releasing.md', 'scripts/legacy-update-bridge.mjs']), true);
  for (const p of ['docs/agent.md', 'docs/AGENTS.md', 'agent.md', '.env', 'docs/secrets.md', 'scripts/publish-release.mjs', 'apps/desktop/src/App.tsx', 'Cargo.lock', '.github/workflows/release.yml']) assert.throws(() => validateBridgeChanges([p]), /unapproved/);
});
test('first publication authenticates the draft, verifies uploaded bytes, then exposes latest; public source/feed stay exact', async () => {
  const f = fixture(), before = clone({ source: f.state.source, channel: f.state.channel, master: f.state.master });
  const result = await f.run(); assert.equal(result.originalBytesUnchanged, true); assert.equal(result.actualProductVersion, '1.0.0-preview.4');
  assert.deepEqual(mutations(f).map(c => c.action), ['draft', 'upload', 'publish-latest']);
  assert.equal(mutations(f)[0].body.make_latest, 'false'); assert.deepEqual(mutations(f).at(-1).body, { draft: false, prerelease: false, make_latest: 'true' });
  const auth = f.calls.findIndex(c => c.action === 'authenticated-draft-read'), publish = f.calls.findIndex(c => c.action === 'publish-latest');
  assert.ok(auth < publish); assert.ok(f.calls.slice(publish + 1).some(c => c.action === 'anonymous-read' && c.url === f.plan.aliasUrl));
  assert.deepEqual({ source: f.state.source, channel: f.state.channel, master: f.state.master }, before);
});
test('interrupted draft with verified asset resumes without a second upload', async () => {
  const f = fixture(); f.completeBridge(true); await f.run(); assert.deepEqual(mutations(f).map(c => c.action), ['publish-latest']);
});
test('empty interrupted draft uploads once and never recreates its release', async () => {
  const f = fixture(); f.completeBridge(true); f.state.bridge.assets = []; await f.run(); assert.deepEqual(mutations(f).map(c => c.action), ['upload', 'publish-latest']);
});
test('published exact bridge is idempotent and does not upload or mutate latest', async () => {
  const f = fixture(); f.completeBridge(false); f.state.latest = clone(f.state.bridge); await f.run(); assert.deepEqual(mutations(f), []);
});
test('a later preview advances from a reviewed prior numeric bridge', async () => {
  const f = fixture('1.0.0-preview.5'); f.request.expectedLatest.tag = 'v1.0.0.4'; f.state.latest.tag_name = 'v1.0.0.4'; f.state.refs['v1.0.0.4'] = f.request.expectedLatest.commit;
  await f.run(); assert.equal(f.state.latest.tag_name, 'v1.0.0.5');
});
test('a different current latest before starting causes no mutation, even for an existing published bridge', async () => {
  for (const published of [false, true]) {
    const f = fixture(); if (published) f.completeBridge(false); f.state.latest = { ...f.state.latest, id: 11, tag_name: 'v1.0.0' };
    await assert.rejects(f.run, /latest release differs/); assert.deepEqual(mutations(f), []);
  }
});
test('latest changing during upload preserves draft and refuses to make it visible', async () => {
  const f = fixture(); f.state.afterUpload = () => { f.state.latest = { ...f.state.latest, id: 33, tag_name: 'v0.7.5' }; };
  await assert.rejects(f.run, /latest release differs/); assert.equal(f.state.bridge.draft, true); assert.ok(!mutations(f).some(c => c.action === 'publish-latest'));
});
test('nonpublished preview, changed full tag source or source asset URL fail before mutations', async () => {
  for (const change of [f => { f.state.source.draft = true; }, f => { f.state.source.prerelease = false; }, f => { f.state.source.target_commitish = 'next'; }, f => { f.state.refs[f.plan.sourceTag] = 'f'.repeat(40); }, f => { f.state.source.assets[0].browser_download_url = f.plan.sourceUrl.replace('https:', 'http:'); }]) {
    const f = fixture(); change(f); await assert.rejects(f.run); assert.deepEqual(mutations(f), []);
  }
});
test('missing/fake digest or wrong source bytes never create a bridge', async () => {
  for (const digest of [null, '', 'SHA256:' + 'a'.repeat(64), 'sha256:' + 'g'.repeat(64), 'sha256:' + 'a'.repeat(63)]) {
    const f = fixture(); f.state.source.assets.find(a => a.name === f.plan.sourceName).digest = digest; await assert.rejects(f.run); assert.deepEqual(mutations(f), []);
  }
  const f = fixture(); f.downloads.set(f.plan.sourceUrl, Buffer.alloc(f.bytes.length)); await assert.rejects(f.run, /bytes\/SHA256 differ/); assert.deepEqual(mutations(f), []);
});
test('existing source preview keeps all four canonical original assets', async () => {
  for (const extra of [false, true]) { const f = fixture(); if (extra) f.state.source.assets.push(f.asset(f.plan.sourceTag, 'private.sig', Buffer.from('x'))); else f.state.source.assets.pop(); await assert.rejects(f.run, /four original/); assert.deepEqual(mutations(f), []); }
});
test('wrong bridge commit/body/digest, extra asset or incomplete published bridge is never overwritten', async () => {
  for (const change of [f => { f.state.bridge.target_commitish = 'next'; }, f => { f.state.bridge.body += ' changed'; }, f => { f.state.bridge.assets[0].digest = 'sha256:' + '0'.repeat(64); }, f => { f.state.bridge.assets.push(f.asset(f.plan.tag, 'junk.sig', Buffer.from('x'))); }, f => { f.state.bridge.draft = false; f.state.bridge.assets = []; f.state.refs[f.plan.tag] = f.plan.targetCommit; }, f => { f.state.refs[f.plan.tag] = 'e'.repeat(40); }]) {
    const f = fixture(); f.completeBridge(true); change(f); await assert.rejects(f.run); assert.deepEqual(mutations(f), []);
  }
});
test('bad authenticated draft bytes or upload digest stay private and never publish', async () => {
  const a = fixture(); a.github.readBridgeAsset = async () => Buffer.alloc(a.bytes.length); await assert.rejects(a.run, /bytes\/SHA256 differ/); assert.equal(a.state.bridge.draft, true);
  const b = fixture(); b.state.afterUpload = () => { b.state.bridge.assets[0].digest = 'sha256:' + '0'.repeat(64); }; await assert.rejects(b.run, /identity differs/); assert.equal(b.state.bridge.draft, true);
  for (const f of [a, b]) assert.ok(!mutations(f).some(c => c.action === 'publish-latest'));
});
test('feed, other preview asset or master mutation during upload blocks exposure', async () => {
  for (const change of [f => { f.state.source.assets[1].digest = 'sha256:' + '0'.repeat(64); }, f => { f.state.channel.assets[0].id++; }, f => { f.state.master = 'f'.repeat(40); }]) {
    const f = fixture(); f.state.afterUpload = () => change(f); await assert.rejects(f.run); assert.equal(f.state.bridge.draft, true); assert.ok(!mutations(f).some(c => c.action === 'publish-latest'));
  }
});
test('runtime/source changes before publication cannot be called a docs-only alias', async () => {
  const f = fixture(); f.state.afterUpload = () => { f.state.invalidSource = true; }; await assert.rejects(f.run, /Runtime changed/); assert.equal(f.state.bridge.draft, true);
});
test('postpublication anonymous byte corruption is detected rather than reported as success', async () => {
  const f = fixture(); f.state.afterPublish = () => { f.downloads.set(f.plan.aliasUrl, Buffer.alloc(f.bytes.length)); };
  await assert.rejects(f.run, /bytes\/SHA256 differ/);
});
