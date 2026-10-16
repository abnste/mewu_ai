// SPDX-License-Identifier: MPL-2.0
// Exercises the production GitHub adapter with isolated transport responses.
// No command in these tests creates a public release, tag or upload.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { releasePlan } from './release-contract.mjs';
import { findRelease, verifyReleaseTarget, verifyStableLatest, publishVersionRelease } from './release-api.mjs';
const repository = 'example/project', tag = 'v1.0.0-preview.4', commit = 'a'.repeat(40);
const draft = { id: 41, tag_name: tag, target_commitish: commit, draft: true, prerelease: true, assets: [] };
function notFound() { return Object.assign(Error('GitHub 404'), { stderr: 'gh: Not Found (HTTP 404)\n' }); }
function transport(routes) {
  const calls = [];
  return { calls, gh(...args) {
    calls.push(args);
    assert.equal(args[0], 'api');
    const route = routes[args[1]];
    if (route instanceof Error) throw route;
    assert.notEqual(route, undefined, `Unexpected API call ${args[1]}`);
    return JSON.stringify(route);
  } };
}
test('published lookup preserves the direct endpoint and needs no draft listing', () => {
  const published = { ...draft, draft: false }, api = transport({ [`repos/${repository}/releases/tags/${tag}`]: published });
  assert.deepEqual(findRelease(api.gh, repository, tag), published);
  assert.equal(api.calls.length, 1);
});
test('new or interrupted draft is recovered across all release pages', () => {
  const api = transport({ [`repos/${repository}/releases/tags/${tag}`]: notFound(), [`repos/${repository}/releases?per_page=100`]: [[{ ...draft, tag_name: 'v0.1.0' }], [draft]] });
  assert.deepEqual(findRelease(api.gh, repository, tag), draft);
  assert.deepEqual(api.calls[1].slice(2), ['--paginate', '--slurp']);
});
test('missing draft returns null; duplicate or malformed listings fail closed', () => {
  for (const [listing, pattern] of [[[], null], [[draft], /Invalid release listing/], [[[draft], [draft]], /Multiple releases/]]) {
    const api = transport({ [`repos/${repository}/releases/tags/${tag}`]: notFound(), [`repos/${repository}/releases?per_page=100`]: listing });
    if (pattern) assert.throws(() => findRelease(api.gh, repository, tag), pattern);
    else assert.equal(findRelease(api.gh, repository, tag), null);
  }
});
test('authentication and transport failures never become missing releases', () => {
  for (const error of [Object.assign(Error('Forbidden'), { stderr: 'gh: Forbidden (HTTP 403)' }), Object.assign(Error('Proxy returned 404 text'), { stderr: 'proxy failed: 404 upstream string' })]) {
    const api = transport({ [`repos/${repository}/releases/tags/${tag}`]: error });
    assert.throws(() => findRelease(api.gh, repository, tag), error);
    assert.equal(api.calls.length, 1);
  }
});
test('draft without a tag requires the exact commit, not a moving branch', () => {
  const api = transport({ [`repos/${repository}/git/ref/tags/${tag}`]: notFound() });
  verifyReleaseTarget(api.gh, repository, draft, tag, commit, true);
  assert.throws(() => verifyReleaseTarget(api.gh, repository, { ...draft, target_commitish: 'next' }, tag, commit, true), /Draft release source differs/);
  assert.throws(() => verifyReleaseTarget(api.gh, repository, { ...draft, draft: false }, tag, commit, true), /Published release tag is missing/);
});
test('published and draft refs must both point directly to the accepted source', () => {
  for (const value of [draft, { ...draft, draft: false }]) {
    const valid = transport({ [`repos/${repository}/git/ref/tags/${tag}`]: { object: { type: 'commit', sha: commit } } });
    verifyReleaseTarget(valid.gh, repository, value, tag, commit, true);
    for (const object of [{ type: 'commit', sha: 'b'.repeat(40) }, { type: 'tag', sha: commit }]) {
      const invalid = transport({ [`repos/${repository}/git/ref/tags/${tag}`]: { object } });
      assert.throws(() => verifyReleaseTarget(invalid.gh, repository, value, tag, commit, true), /tag source differs/);
    }
  }
});
test('tag and preview identity cannot be changed while recovering a draft', () => {
  const api = transport({});
  for (const value of [null, { ...draft, tag_name: 'v9.0.0' }, { ...draft, prerelease: false }, { ...draft, draft: 'true' }]) assert.throws(() => verifyReleaseTarget(api.gh, repository, value, tag, commit, true), /identity differs/);
  assert.equal(api.calls.length, 0);
});

test('real stable release must be GitHub latest even when an archived numeric bridge is higher', async () => {
  const stable = { ...draft, id: 100, tag_name: 'v1.0.0', draft: false, prerelease: false };
  const valid = transport({ [`repos/${repository}/releases/latest`]: stable });
  assert.deepEqual(await verifyStableLatest(valid.gh, repository, stable), stable);
  for (const latest of [null, { ...stable, id: 101, tag_name: 'v1.0.0.4' }, { ...stable, id: 101 }, { ...stable, draft: true }, { ...stable, prerelease: true }]) {
    const invalid = transport({ [`repos/${repository}/releases/latest`]: latest });
    await assert.rejects(verifyStableLatest(invalid.gh, repository, stable, { pause: async () => {} }), /latest does not identify/);
    assert.equal(invalid.calls.length, 4);
  }
});

test('latest verification refuses a draft or preview expectation without any API read', async () => {
  const api = transport({});
  for (const expected of [null, draft, { ...draft, draft: false }, { ...draft, id: 0, draft: false, prerelease: false }]) {
    await assert.rejects(verifyStableLatest(api.gh, repository, expected), /Published stable release required/);
  }
  assert.equal(api.calls.length, 0);
});

test('latest visibility delay retries only reads with a fixed short bound', async () => {
  const stable = { ...draft, id: 100, tag_name: 'v1.0.0', draft: false, prerelease: false };
  const bridge = { ...stable, id: 99, tag_name: 'v1.0.0.4' }, pauses = [];
  let reads = 0;
  const gh = (...args) => {
    assert.deepEqual(args, ['api', `repos/${repository}/releases/latest`]);
    return JSON.stringify(++reads < 3 ? bridge : stable);
  };
  assert.deepEqual(await verifyStableLatest(gh, repository, stable, { pause: async ms => { pauses.push(ms); } }), stable);
  assert.equal(reads, 3); assert.deepEqual(pauses, [500, 500]);
});

function modernFixture({ version = '1.0.0', existing, latest, source = commit } = {}) {
  const plan = releasePlan(version), names = [...Object.values(plan).flatMap(p => p.files), plan['darwin-aarch64'].updater, 'latest.json'];
  const artifacts = names.map((name, index) => ({ name, bytes: index + 10, sha256: String(index).repeat(64), path: path.join('fixture', name) }));
  const asset = name => { const row = artifacts.find(a => a.name === name); return { id: names.indexOf(name) + 500, name, size: row.bytes, digest: `sha256:${row.sha256}` }; };
  let current = existing ? { id: 100, tag_name: `v${version}`, target_commitish: commit, prerelease: version.includes('-'), assets: [], ...existing } : null;
  let stable = latest ?? { id: 1, tag_name: 'v0.7.4', draft: false, prerelease: false };
  const calls = [], mutations = [];
  const gh = (...args) => {
    calls.push(args); assert.ok(!args.some(value => String(value).includes('update-channel')), 'Modern publication must not access legacy infrastructure');
    if (args[0] === 'api') {
      const endpoint = args[1];
      if (endpoint.endsWith('/releases/latest')) return JSON.stringify(stable);
      if (endpoint.endsWith(`/releases/tags/v${version}`)) { if (!current || current.draft) throw notFound(); return JSON.stringify(current); }
      if (endpoint.endsWith('/releases?per_page=100')) return JSON.stringify([current ? [current] : []]);
      if (endpoint.endsWith(`/git/ref/tags/v${version}`)) { if (!current) throw notFound(); return JSON.stringify({ object: { type: 'commit', sha: source } }); }
      throw Error(`Unexpected fixture endpoint ${endpoint}`);
    }
    mutations.push(args); assert.equal(args[0], 'release'); assert.equal(args[2], `v${version}`);
    if (args[1] === 'create') current = { id: 100, tag_name: `v${version}`, target_commitish: commit, draft: true, prerelease: args.includes('--prerelease'), assets: [] };
    else if (args[1] === 'upload') {
      assert.ok(current.draft, 'Never mutate published packages'); assert.ok(!args.includes('--clobber'));
      current.assets.push(asset(path.basename(args.at(-1))));
    } else if (args[1] === 'edit') {
      assert.equal(current.assets.length, 6, 'Publish only after all six assets are uploaded');
      current.draft = false; if (args.includes('--latest')) stable = current;
    } else throw Error('Unexpected fixture mutation');
    return '';
  };
  let localChecks = 0;
  const run = options => publishVersionRelease(gh, repository, { version, commit, artifacts, notes: 'fixture-notes.md', verifyLocal: () => { localChecks++; }, pause: async () => {}, ...options });
  return { run, artifacts, asset, calls, mutations, get current() { return current; }, get localChecks() { return localChecks; } };
}
test('modern stable uploads all six version-owned assets before Latest without any channel request', async () => {
  const fixture = modernFixture();
  const result = await fixture.run();
  assert.equal(result.draft, false); assert.equal(result.assets.length, 6);
  assert.equal(fixture.mutations.filter(call => call[1] === 'upload').length, 6);
  assert.ok(result.assets.some(asset => asset.name === 'latest.json'));
  assert.ok(result.assets.some(asset => asset.name.endsWith('.app.tar.gz')));
  assert.ok(fixture.mutations.at(-1).includes('--latest')); assert.ok(fixture.localChecks >= 8);
});
test('modern partial draft resumes only missing uploads and preserves exact original assets', async () => {
  const fixture = modernFixture({ existing: { draft: true } });
  fixture.current.assets.push(fixture.asset(fixture.artifacts[0].name));
  await fixture.run();
  assert.equal(fixture.mutations.some(call => call[1] === 'create'), false);
  assert.equal(fixture.mutations.filter(call => call[1] === 'upload').length, 5);
});
test('published same-version bytes and source remain immutable even for stable 1.0.0', async () => {
  for (const kind of ['changed', 'incomplete', 'source', 'duplicate']) {
    const fixture = modernFixture({ existing: { draft: false }, ...(kind === 'source' ? { source: 'b'.repeat(40) } : {}) });
    fixture.current.assets = fixture.artifacts.map(a => fixture.asset(a.name));
    if (kind === 'changed') fixture.current.assets[0].digest = `sha256:${'f'.repeat(64)}`;
    if (kind === 'incomplete') fixture.current.assets.pop();
    if (kind === 'duplicate') fixture.current.assets[5] = { ...fixture.current.assets[0] };
    await assert.rejects(fixture.run(), /differs|incomplete|duplicate/i);
    assert.equal(fixture.mutations.length, 0);
  }
});
test('already-published exact modern release is a read-only idempotent publication', async () => {
  const fixture = modernFixture({ existing: { draft: false }, latest: { id: 100, tag_name: 'v1.0.0', draft: false, prerelease: false } });
  fixture.current.assets = fixture.artifacts.map(a => fixture.asset(a.name));
  await fixture.run(); assert.equal(fixture.mutations.length, 0);
});
test('modern preview remains independent but does not promote stable Latest or write legacy feeds', async () => {
  const fixture = modernFixture({ version: '1.0.1-preview.1' });
  await fixture.run();
  assert.ok(fixture.mutations.at(-1).includes('--latest=false'));
  assert.equal(fixture.calls.some(call => call[1]?.endsWith('/releases/latest')), false);
});
test('modern downgrade and incomplete six-asset input fail before remote mutation', async () => {
  const downgrade = modernFixture({ latest: { id: 2, tag_name: 'v1.1.0', draft: false, prerelease: false } });
  await assert.rejects(downgrade.run(), /downgrade/); assert.equal(downgrade.mutations.length, 0);
  const missing = modernFixture(); await assert.rejects(missing.run({ artifacts: missing.artifacts.slice(0, 5) }), /Exactly six/); assert.equal(missing.calls.length, 0);
});
test('modern local acceptance failure cannot create a draft or upload', async () => {
  const fixture = modernFixture();
  await assert.rejects(fixture.run({ verifyLocal: () => { throw Error('Local acceptance changed'); } }), /Local acceptance changed/);
  assert.equal(fixture.calls.length, 0);
});
