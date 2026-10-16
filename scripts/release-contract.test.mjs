// SPDX-License-Identifier: MPL-2.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { platforms, requiredChecks, releasePlan, updateUrl, fingerprint, verifyRelease, verifySignatureVersion, releaseVersion, compareReleaseVersions, validateChannelAdvance, publicationPolicy, validateUpdateConfiguration, selfContainedRelease, validateManifest, validateLegacyManifest, legacyUpdateUrl, githubReleaseUpdates, composeReleaseNotes, validateReleaseNotes, releaseSignatureMarker } from './release-contract.mjs';
import { compareLegacyVersions } from './legacy-update-bridge.mjs';
const version = '1.0.0-preview.3', commit = 'a'.repeat(40);
const signature = Buffer.from(`untrusted comment: fixture\nfixture\ntrusted comment: timestamp:1\tversion:${version}\nfixture\n`).toString('base64');
function fixture(callback, candidateVersion = version) {
  const version = candidateVersion;
  const signature = Buffer.from(`untrusted comment: fixture\nfixture\ntrusted comment: timestamp:1\tversion:${version}\nfixture\n`).toString('base64');
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'mewu-release-gate-'));
  try {
    const plan = releasePlan(version), manifest = { version, platforms: {} }, acceptance = { sourceCommit: commit, version, platforms: {} };
    fs.mkdirSync(path.join(directory, '.metadata'));
    for (const platform of platforms) {
      const names = [...new Set([...plan[platform].files, plan[platform].updater])];
      for (const name of names) fs.writeFileSync(path.join(directory, name), `artifact ${platform} ${name}`);
      manifest.platforms[platform] = { signature, url: updateUrl(version, platform) };
      acceptance.platforms[platform] = { schemaVersion: 1, platform, version, sourceCommit: commit, result: 'passed', nativeGuiVerified: true, installerVerified: true, nativeTests: { passed: 10, failed: 0 }, frontendTests: { passed: 10, failed: 0 }, checks: Object.fromEntries(requiredChecks.map(key => [key, 'passed'])), artifacts: names.map(name => fingerprint(path.join(directory, name))) };
    }
    fs.writeFileSync(path.join(directory, '.metadata', 'update.json'), JSON.stringify(manifest));
    fs.writeFileSync(path.join(directory, '.metadata', 'acceptance.json'), JSON.stringify(acceptance));
    if (selfContainedRelease(version) && !githubReleaseUpdates(version)) fs.copyFileSync(path.join(directory, '.metadata', 'update.json'), path.join(directory, 'latest.json'));
    callback({ directory, manifest, acceptance, plan });
  } finally { fs.rmSync(directory, { recursive: true }); }
}
test('three native receipts bind installer and update bytes; mac update archive remains in channel', () => fixture(({ directory, manifest, acceptance }) => {
  const result = verifyRelease(directory, manifest, acceptance, commit);
  assert.equal(result.assets.length, 4);
  assert.deepEqual(result.channelAssets, [`MewuAI-Remake-Update-${version}-mac-arm64.app.tar.gz`]);
  assert.ok(result.assets.every(name => !name.endsWith('.sig')));
}));
for (const platform of platforms) {
  test(`${platform}: missing receipt refuses publication`, () => fixture(({ directory, manifest, acceptance }) => {
    delete acceptance.platforms[platform]; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
  }));
  for (const field of ['nativeGuiVerified', 'installerVerified']) test(`${platform}: regression success cannot replace ${field}`, () => fixture(({ directory, manifest, acceptance }) => {
    acceptance.platforms[platform][field] = false; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
  }));
  for (const check of requiredChecks) test(`${platform}: ${check} pending refuses publication`, () => fixture(({ directory, manifest, acceptance }) => {
    acceptance.platforms[platform].checks[check] = 'pending'; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
  }));
  test(`${platform}: different source/version and failed suite refuse`, () => fixture(({ directory, manifest, acceptance }) => {
    const original = structuredClone(acceptance.platforms[platform]);
    for (const mutation of [r => r.sourceCommit = 'b'.repeat(40), r => r.version = '1.0.0-preview.2', r => r.nativeTests.failed = 1, r => r.frontendTests.passed = 0, r => r.artifacts[0].bytes += 1]) {
      acceptance.platforms[platform] = structuredClone(original); mutation(acceptance.platforms[platform]);
      assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
    }
  }));
  test(`${platform}: changed artifact refuses`, () => fixture(({ directory, manifest, acceptance, plan }) => {
    fs.appendFileSync(path.join(directory, plan[platform].updater), 'tampered');
    assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
  }));
  test(`${platform}: cross-platform and alternate-host update URL refuse`, () => fixture(({ directory, manifest, acceptance }) => {
    const value = manifest.platforms[platform];
    for (const url of [updateUrl(version, platforms.find(p => p !== platform)), value.url.replace('github.com', 'example.com'), `${value.url}?download=1`, value.url.replace(version, '1.0.0-preview.2')]) {
      value.url = url; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
    }
  }));
}
test('missing third platform update refuses', () => fixture(({ directory, manifest, acceptance }) => {
  delete manifest.platforms['linux-x86_64']; assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
}));
test('signature version is required exactly once', () => {
  verifySignatureVersion(signature, version);
  for (const text of ['', 'trusted comment: version:1.0.0-preview.2', `trusted comment: version:${version}\tversion:${version}`, `trusted comment: timestamp:1`]) assert.throws(() => verifySignatureVersion(Buffer.from(text).toString('base64'), version));
});
for (const name of ['private-key.pem', 'debug.log', 'agent.md', 'extra.sig']) test(`unallowlisted ${name} refuses`, () => fixture(({ directory, manifest, acceptance }) => {
  fs.writeFileSync(path.join(directory, name), 'fixture'); assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
}));
test('unexpected metadata refuses', () => fixture(({ directory, manifest, acceptance }) => {
  fs.writeFileSync(path.join(directory, '.metadata', 'extra.json'), '{}'); assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
}));
test('unchanged filenames but replaced updater bytes cannot reuse old receipt', () => fixture(({ directory, manifest, acceptance, plan }) => {
  const target = path.join(directory, plan['darwin-aarch64'].updater), original = fs.readFileSync(target);
  const changed = Buffer.from(original); changed[0] ^= 1; fs.writeFileSync(target, changed);
  assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
}));
test('SemVer restricted grammar rejects leading zeros, build suffixes and oversized input', () => {
  for (const value of ['1.01.0', '1.0.01', '1.0.0-preview.03', '1.0.0+private', '1.0.0-preview.' + '1'.repeat(70)]) assert.throws(() => releaseVersion(value));
});
test('channel precedence handles numeric prereleases and release stability', () => {
  for (const [low, high] of [['1.0.0-preview.2', '1.0.0-preview.10'], ['1.0.0-rc.9', '1.0.0'], ['1.9.0', '1.10.0'], ['1.0.0-alpha.2', '1.0.0-beta.1'], ['1.0.0', '1.0.1-alpha.1']]) {
    assert.equal(compareReleaseVersions(low, high), -1); assert.equal(compareReleaseVersions(high, low), 1);
  }
  assert.equal(compareReleaseVersions(version, version), 0);
});
test('update channel cannot downgrade or change same-version bytes', () => fixture(({ manifest }) => {
  validateChannelAdvance(manifest, structuredClone(manifest));
  assert.throws(() => validateChannelAdvance({ version: '1.0.0' }, manifest));
  const changed = structuredClone(manifest); changed.platforms['darwin-aarch64'].signature += 'A';
  assert.throws(() => validateChannelAdvance(manifest, changed));
  validateChannelAdvance({ version: '1.0.0-preview.2', platforms: {} }, manifest);
}));

test('stable 1.0.0 needs matching stable packages, native receipts and signature version', () => fixture(({ directory, manifest, acceptance }) => {
  const release = verifyRelease(directory, manifest, acceptance, commit);
  assert.equal(release.tag, 'v1.0.0'); assert.equal(release.assets.length, 6);
  assert.deepEqual(release.channelAssets, []); assert.equal(release.softwareAssets.length, 4);
  assert.ok(release.assets.every(name => name === 'latest.json' || name.includes('1.0.0-') && !name.includes('preview') && !name.endsWith('.sig')));
  const oldSignature = Buffer.from('untrusted comment: fixture\nfixture\ntrusted comment: version:1.0.0-preview.4\nfixture\n').toString('base64');
  manifest.platforms['windows-x86_64'].signature = oldSignature;
  assert.throws(() => verifyRelease(directory, manifest, acceptance, commit), /Signature does not bind/);
}, '1.0.0'));

test('stable publication selects GitHub Latest and owns its signed feed and updater', () => {
  const stable = publicationPolicy('1.0.0');
  assert.equal(stable.prerelease, false); assert.deepEqual(stable.latestArguments, ['--latest']);
  assert.deepEqual(stable.feeds, []);
  assert.equal(stable.clientEndpoint, 'https://github.com/abnste/mewu_ai/releases/latest/download/latest.json');
  assert.deepEqual(publicationPolicy('1.0.1').feeds, stable.feeds);
  assert.equal(publicationPolicy('1.0.1').clientEndpoint, 'https://api.github.com/repos/abnste/mewu_ai/releases/latest');
  const preview = publicationPolicy('1.1.0-preview.1');
  assert.equal(preview.prerelease, true); assert.deepEqual(preview.latestArguments, ['--latest=false']);
  assert.deepEqual(preview.feeds, []); assert.equal(preview.clientEndpoint, publicationPolicy('1.0.1').clientEndpoint);
  const historical = publicationPolicy('1.0.0-preview.4');
  assert.deepEqual(historical.feeds, ['preview.json']); assert.ok(historical.clientEndpoint.endsWith('/update-channel/preview.json'));
});

test('source version must select its own fixed signed channel before staging or publication', () => {
  for (const version of ['1.0.0-preview.4', '1.0.0', '1.0.1']) {
    const config = { version, plugins: { updater: { endpoints: [publicationPolicy(version).clientEndpoint], requireSignedVersion: true } } };
    assert.deepEqual(validateUpdateConfiguration(config), publicationPolicy(version));
    for (const mutate of [c => c.plugins.updater.requireSignedVersion = false, c => c.plugins.updater.endpoints.reverse().push('https://example.com/feed'), c => c.plugins.updater.endpoints[0] = publicationPolicy(selfContainedRelease(version) ? '1.0.0-preview.4' : '1.0.0').clientEndpoint]) {
      const wrong = structuredClone(config); mutate(wrong);
      assert.throws(() => validateUpdateConfiguration(wrong), /fixed signed release channel/);
    }
  }
});

test('layout cutover preserves historical previews and makes every stable self-contained', () => {
  for (const value of ['1.0.0', '1.0.1', '1.1.0', '1.0.1-preview.1', '1.1.0-alpha.0']) assert.equal(selfContainedRelease(value), true);
  for (const value of ['1.0.0-preview.4', '1.0.0-alpha.0', '1.0.0-rc.1']) assert.equal(selfContainedRelease(value), false);
});
test('modern and legacy Mac URLs remain distinct and strictly validated', () => fixture(({ manifest }) => {
  const modern = manifest.platforms['darwin-aarch64'].url;
  assert.ok(modern.includes('/v1.0.0/')); assert.equal(validateManifest(manifest), '1.0.0');
  assert.throws(() => validateLegacyManifest(manifest), /Invalid platform update URL/);
  manifest.platforms['darwin-aarch64'].url = legacyUpdateUrl('1.0.0', 'darwin-aarch64');
  assert.equal(validateLegacyManifest(manifest), '1.0.0');
  assert.throws(() => validateManifest(manifest), /Invalid platform update URL/);
}, '1.0.0'));
for (const mutation of ['missing', 'different', 'format']) test(`modern signed public feed ${mutation} refuses`, () => fixture(({ directory, manifest, acceptance }) => {
  const file = path.join(directory, 'latest.json');
  if (mutation === 'missing') fs.unlinkSync(file);
  else fs.writeFileSync(file, mutation === 'format' ? JSON.stringify(manifest, null, 2) : JSON.stringify({ ...manifest, notes: 'changed' }));
  assert.throws(() => verifyRelease(directory, manifest, acceptance, commit));
}, '1.0.0'));

test('legacy numeric bridge is not a product SemVer or the installed legacy current version', () => {
  // Original dotnet ApplicationUpdateService.cs compares only latestVersion to
  // currentVersion (the installed assembly), never to a previously seen tag.
  assert.equal(compareLegacyVersions('v1.0.0.4', 'v1.0.0'), 1);
  assert.equal(compareLegacyVersions('v1.0.0', 'v0.7.4.0'), 1);
  assert.equal(compareReleaseVersions('1.0.0', '1.0.0-preview.4'), 1);
  assert.throws(() => releaseVersion('1.0.0.4'), /Invalid release version/);
  assert.deepEqual(publicationPolicy('1.0.0').latestArguments, ['--latest']);
});
test('new releases use product names and only five payloads; feed/signature files refuse', () => fixture(({directory,manifest,acceptance})=>{
  const release=verifyRelease(directory,manifest,acceptance,commit);
  assert.equal(release.assets.length,5);
  assert.ok(release.assets.every(name=>!name.includes('Remake')&&!name.endsWith('.json')&&!name.endsWith('.sig')));
  assert.ok(release.assets.includes('MewuAI-Setup-1.0.1-win-x64.exe'));
  assert.deepEqual(release.channelAssets,[]);
  fs.writeFileSync(path.join(directory,'latest.json'),'{}');
  assert.throws(()=>verifyRelease(directory,manifest,acceptance,commit),/Unexpected release artifact/);
},'1.0.1'));
test('GitHub release body signatures bind exact version, unique target names and accepted signatures',()=>fixture(({manifest})=>{
  const notes=composeReleaseNotes('# MewuAI 1.0.1\n\nChanges.',manifest);
  const value=validateReleaseNotes(notes,'1.0.1',manifest);
  assert.equal(value.assets.length,3);
  assert.ok(notes.startsWith('# MewuAI 1.0.1\n\nChanges.\n\n<!--'));
  assert.throws(()=>composeReleaseNotes(notes,manifest),/Invalid original/);
  for(const mutate of [v=>v.version='1.0.2',v=>v.extra=true,v=>v.assets.pop(),v=>v.assets[1]={...v.assets[0]},v=>v.assets[0].name='other.exe',v=>v.assets[0].url='https://example.com',v=>v.assets[0].signature='invalid']){
    const bad=structuredClone(value);mutate(bad);
    assert.throws(()=>validateReleaseNotes(`${releaseSignatureMarker}${JSON.stringify(bad)}\n-->`,'1.0.1',manifest));
  }
  for(const bad of ['',notes+notes,notes.replace('v1\n','v2\n'),notes.replace('\n-->',''),notes.replace('"schemaVersion":1','"schemaVersion":')]) assert.throws(()=>validateReleaseNotes(bad,'1.0.1',manifest));
},'1.0.1'));
