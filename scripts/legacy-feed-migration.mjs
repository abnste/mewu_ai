// SPDX-License-Identifier: MPL-2.0
// One-time old-client bridge. Modern publication never imports this module.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { fingerprint, platforms, releasePlan, legacyUpdateUrl, validateManifest, validateLegacyManifest, validateChannelAdvance } from './release-contract.mjs';
import { findRelease, verifyReleaseTarget } from './release-api.mjs';
const version = '1.0.0', tag = `v${version}`, tar = `MewuAI-Remake-Update-${version}-mac-arm64.app.tar.gz`;
const feedNames = ['preview.json', 'latest.json'];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const hashPattern = /^[a-f0-9]{64}$/;
export function legacyRepairManifest(manifest) {
  if (manifest?.version !== version) throw Error('Legacy repair is fixed to stable 1.0.0');
  validateManifest(manifest);
  const legacy = structuredClone(manifest);
  for (const platform of platforms) legacy.platforms[platform].url = legacyUpdateUrl(version, platform);
  validateLegacyManifest(legacy);
  return legacy;
}
// Ordinary advancement stays strict. Only the publisher's exact, hash-bound
// repair authorization below permits replacing the explicitly reviewed 1.0.0.
export function validateLegacyMigrationAdvance(current, candidate) {
  validateLegacyManifest(current); validateLegacyManifest(candidate);
  validateChannelAdvance(current, candidate);
  if (current.version === candidate.version && !isDeepStrictEqual(current, candidate)) throw Error('Same-version legacy migration manifest differs');
}
export function publishLegacyRepair(gh, repository, { manifest, commit, artifacts, feedFiles, verifyLocal, repairAuthorization }) {
  const legacy = legacyRepairManifest(manifest);
  if (!/^[a-f0-9]{40}$/.test(commit) || typeof verifyLocal !== 'function') throw Error('Accepted source required for legacy repair');
  const plan = releasePlan(version), names = [...Object.values(plan).flatMap(p => p.files), tar, 'latest.json'];
  if (!Array.isArray(artifacts) || artifacts.length !== names.length || new Set(artifacts.map(a => a.name)).size !== names.length || names.some(name => !artifacts.some(a => a.name === name))) throw Error('Six original formal assets required');
  const expected = new Map(artifacts.map(a => [a.name, a])), mirror = expected.get(tar);
  const sameFingerprint = (asset, wanted) => Boolean(wanted && asset.name === wanted.name && asset.size === wanted.bytes && asset.digest === `sha256:${wanted.sha256}`);
  const validatePrevious = value => value && Number.isSafeInteger(value.id) && value.id > 0 && Number.isSafeInteger(value.bytes) && value.bytes > 0 && hashPattern.test(value.sha256);
  const authorization = repairAuthorization;
  if (authorization) {
    if (authorization.schemaVersion !== 1 || authorization.version !== version || authorization.sourceCommit !== commit || !Number.isSafeInteger(authorization.channelReleaseId) || authorization.channelReleaseId < 1 || authorization.targetManifestSha256 !== expected.get('latest.json').sha256 || authorization.targetUpdaterSha256 !== mirror.sha256 || !feedNames.every(name => validatePrevious(authorization.previousFeeds?.[name])) || !validatePrevious(authorization.previousUpdater) || authorization.previousUpdater.name !== tar) throw Error('Invalid exact legacy repair authorization');
  }
  const targetFeeds = {};
  for (const name of feedNames) {
    if (!feedFiles?.[name] || path.basename(feedFiles[name]) !== name) throw Error('Missing legacy feed file');
    const raw = fs.readFileSync(feedFiles[name]);
    if (!raw.length || raw.length > 64 * 1024 || !isDeepStrictEqual(JSON.parse(raw.toString('utf8')), legacy)) throw Error('Legacy feed file differs from repair manifest');
    targetFeeds[name] = { raw, sha256: digest(raw) };
  }
  if (!targetFeeds['latest.json'].raw.equals(targetFeeds['preview.json'].raw)) throw Error('Legacy feeds must contain identical derived bytes');
  function checkAsset(asset, wanted) {
    if (!sameFingerprint(asset, wanted)) throw Error('Formal migration asset differs');
  }
  function oldAssetMatches(asset, previous) {
    return validatePrevious(previous) && asset.id === previous.id && asset.size === previous.bytes && asset.digest === `sha256:${previous.sha256}`;
  }
  const lookup = name => findRelease(gh, repository, name);
  const validateChannel = current => {
    if (!current || current.tag_name !== 'update-channel' || current.draft !== false || current.prerelease !== true || (authorization && current.id !== authorization.channelReleaseId)) throw Error('Invalid existing legacy update channel');
  };
  function readFeeds(current) {
    validateChannel(current);
    const states = {};
    for (const name of feedNames) {
      const found = current.assets.filter(a => a.name === name);
      if (found.length !== 1 || found[0].size > 64 * 1024) throw Error('Missing, duplicate or oversized legacy feed');
      const asset = found[0], raw = Buffer.from(gh('api', `repos/${repository}/releases/assets/${asset.id}`, '--header', 'Accept: application/octet-stream'));
      if (raw.length !== asset.size || raw.length > 64 * 1024 || asset.digest !== `sha256:${digest(raw)}`) throw Error('Legacy feed bytes differ from metadata');
      const value = JSON.parse(raw.toString('utf8')); validateLegacyManifest(value);
      const target = raw.equals(targetFeeds[name].raw);
      if (!target) {
        if (value.version === version && !authorization) throw Error('Same-version legacy feed requires exact repair authorization');
        try { validateLegacyMigrationAdvance(value, legacy); }
        catch (error) {
          if (value.version !== version || !authorization || !oldAssetMatches(asset, authorization.previousFeeds[name]) || digest(raw) !== authorization.previousFeeds[name].sha256) throw error;
        }
        if (authorization && !oldAssetMatches(asset, authorization.previousFeeds[name])) throw Error('Legacy feed baseline changed');
      }
      states[name] = { asset, target };
    }
    return states;
  }
  function readMirror(current) {
    const found = current.assets.filter(a => a.name === tar);
    if (found.length !== 1) throw Error('Existing legacy payload missing or duplicated');
    const asset = found[0];
    if (sameFingerprint(asset, mirror)) return { asset, target: true };
    if (!authorization || !oldAssetMatches(asset, authorization.previousUpdater)) throw Error('Same-version legacy payload requires exact repair authorization');
    return { asset, target: false };
  }
  verifyLocal();
  const published = lookup(tag);
  verifyReleaseTarget(gh, repository, published, tag, commit, false);
  if (published.draft || published.assets.length !== expected.size || new Set(published.assets.map(a => a.name)).size !== expected.size) throw Error('Formal 1.0.0 must already be fully published');
  for (const asset of published.assets) checkAsset(asset, expected.get(asset.name));
  let channel = lookup('update-channel');
  readFeeds(channel); const beforeMirror = readMirror(channel);
  if (!beforeMirror.target) {
    verifyLocal();
    const actual = fingerprint(mirror.path);
    if (actual.sha256 !== mirror.sha256 || actual.bytes !== mirror.bytes) throw Error('Accepted repair payload changed');
    channel = lookup('update-channel'); readFeeds(channel);
    if (!readMirror(channel).target) gh('release', 'upload', 'update-channel', '--repo', repository, '--clobber', mirror.path);
  }
  channel = lookup('update-channel'); validateChannel(channel);
  if (!readMirror(channel).target) throw Error('Repaired migration mirror missing');
  const changed = [];
  for (const name of feedNames) {
    channel = lookup('update-channel'); const states = readFeeds(channel);
    if (!readMirror(channel).target) throw Error('Migration mirror changed');
    if (!states[name].target) {
      verifyLocal();
      if (!fs.readFileSync(feedFiles[name]).equals(targetFeeds[name].raw)) throw Error('Prepared legacy feed changed');
      gh('release', 'upload', 'update-channel', '--repo', repository, '--clobber', feedFiles[name]); changed.push(name);
    }
  }
  channel = lookup('update-channel'); const final = readFeeds(channel);
  if (!Object.values(final).every(state => state.target) || !readMirror(channel).target) throw Error('Legacy repair publication incomplete');
  return { version, sourceCommit: commit, legacyFeedsUpdated: changed, mirror: tar, formalReleaseModified: false, originalSignatureRetained: true, sameVersionInstalledStableAutoUpdate: false };
}
