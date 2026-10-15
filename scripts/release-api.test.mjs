// SPDX-License-Identifier: MPL-2.0
// Exercises the production GitHub adapter with isolated transport responses.
// No command in these tests creates a public release, tag or upload.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { findRelease, verifyReleaseTarget } from './release-api.mjs';
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
