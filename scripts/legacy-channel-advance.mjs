// SPDX-License-Identifier: MPL-2.0
// Optional migration for old clients. Ordinary stable publication is independent.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import {
  repository as officialRepository, platforms, publicationPolicy, releasePlan,
  compareReleaseVersions, legacyUpdateUrl, validateManifest,
  validateLegacyManifest, validateChannelAdvance, fingerprint,
} from './release-contract.mjs';
import { findRelease, verifyReleaseTarget } from './release-api.mjs';

const feeds = ['preview.json', 'latest.json'];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const assert = (value, message) => { if (!value) throw Error(message); };
const sameAsset = (asset, file) => Boolean(asset && file && asset.name === file.name
  && asset.size === file.bytes && asset.digest === `sha256:${file.sha256}`);
const identity = asset => asset ? [asset.id, asset.name, asset.size, asset.digest] : null;

export function legacyAdvanceManifest(manifest) {
  const policy = publicationPolicy(manifest?.version);
  assert(!policy.prerelease && policy.selfContained
    && compareReleaseVersions(manifest.version, '1.0.0') > 0,
  'Ordinary legacy advancement requires a stable version newer than 1.0.0');
  validateManifest(manifest);
  const legacy = structuredClone(manifest);
  for (const platform of platforms) legacy.platforms[platform].url = legacyUpdateUrl(manifest.version, platform);
  validateLegacyManifest(legacy);
  return legacy;
}

/** downloadAsset(asset, destination) must download that exact published asset ID.
 * A failed write is never retried here, even if the server may have completed it.
 * An explicit later invocation can inspect and resume an exact target state.
 */
export function publishLegacyAdvance(gh, repository, {
  manifest, commit, artifacts, feedFiles, workDirectory, verifyLocal,
  downloadAsset, onProgress = () => {},
}) {
  const legacy = legacyAdvanceManifest(manifest), version = manifest.version, tag = `v${version}`;
  assert(repository === officialRepository && /^[a-f0-9]{40}$/.test(commit)
    && typeof verifyLocal === 'function' && typeof downloadAsset === 'function', 'Exact official accepted source and download transport required');
  const plan = releasePlan(version), tar = plan['darwin-aarch64'].updater;
  const names = [...Object.values(plan).flatMap(entry => entry.files), tar, 'latest.json'];
  assert(Array.isArray(artifacts) && artifacts.length === 6
    && new Set(artifacts.map(file => file.name)).size === 6
    && names.every(name => artifacts.some(file => file.name === name)), 'Exactly six accepted formal assets required');
  const expected = new Map(artifacts.map(file => [file.name, file])), mirror = expected.get(tar);
  const directory = fs.realpathSync(workDirectory);
  assert(fs.lstatSync(workDirectory).isDirectory() && !fs.lstatSync(workDirectory).isSymbolicLink()
    && fs.readdirSync(directory).length === 0, 'Fresh private download directory required');
  const targetRaw = {}, changes = [], downloads = [], writes = [];
  for (const name of feeds) {
    assert(feedFiles?.[name] && path.basename(feedFiles[name]) === name, 'Exact legacy feed files required');
    const raw = fs.readFileSync(feedFiles[name]);
    assert(raw.length > 0 && raw.length <= 64 * 1024
      && isDeepStrictEqual(JSON.parse(raw.toString('utf8')), legacy), 'Derived legacy feed differs');
    targetRaw[name] = raw;
  }
  assert(targetRaw['preview.json'].equals(targetRaw['latest.json']), 'Derived legacy feeds must be byte identical');
  const lookup = value => findRelease(gh, repository, value);
  let publishedId, channelId, preserved, mirrorVerified;
  const progress = extra => onProgress({ version, sourceCommit: commit, ...extra });
  function formal() {
    verifyLocal();
    const release = lookup(tag);
    verifyReleaseTarget(gh, repository, release, tag, commit, false);
    assert(release.draft === false && Number.isSafeInteger(release.id)
      && (!publishedId || release.id === publishedId), 'Formal release must already be public and unchanged');
    assert(Array.isArray(release.assets) && release.assets.length === 6
      && new Set(release.assets.map(asset => asset.name)).size === 6, 'Public formal release must contain exactly six assets');
    for (const asset of release.assets) {
      assert(Number.isSafeInteger(asset.id) && sameAsset(asset, expected.get(asset.name))
        && asset.browser_download_url === `https://github.com/${repository}/releases/download/${tag}/${asset.name}`, 'Published formal asset identity differs');
    }
    const latest = JSON.parse(gh('api', `repos/${repository}/releases/latest`));
    assert(latest?.id === release.id && latest.tag_name === tag
      && latest.draft === false && latest.prerelease === false, 'Candidate must remain the current public stable release');
    publishedId = release.id;
    return release;
  }
  function download(asset, prefix) {
    const destination = path.join(directory, `${prefix}-${asset.id}-${asset.name}`);
    assert(!fs.existsSync(destination), 'Never overwrite verification downloads');
    downloadAsset(asset, destination);
    const actual = fingerprint(destination);
    assert(actual.bytes === asset.size && `sha256:${actual.sha256}` === asset.digest, 'Downloaded published asset bytes differ');
    downloads.push({ assetId: asset.id, name: asset.name, bytes: actual.bytes, sha256: actual.sha256 });
    return { destination, actual };
  }
  function channel() {
    const release = lookup('update-channel');
    assert(release && release.tag_name === 'update-channel' && release.draft === false
      && release.prerelease === true && Number.isSafeInteger(release.id)
      && (!channelId || release.id === channelId), 'Existing legacy channel identity differs');
    assert(Array.isArray(release.assets) && new Set(release.assets.map(asset => asset.name)).size === release.assets.length, 'Duplicate channel asset');
    const others = release.assets.filter(asset => !feeds.includes(asset.name) && asset.name !== tar)
      .map(identity).sort((a, b) => a[1].localeCompare(b[1]));
    assert(!preserved || isDeepStrictEqual(others, preserved), 'Unrelated legacy assets changed; stop without touching them');
    preserved ??= others; channelId = release.id;
    return release;
  }
  function feedStates(release) {
    return Object.fromEntries(feeds.map(name => {
      const asset = release.assets.find(value => value.name === name);
      assert(asset && Number.isSafeInteger(asset.id) && asset.size > 0 && asset.size <= 64 * 1024, 'Legacy feed missing or oversized');
      const raw = Buffer.from(gh('api', `repos/${repository}/releases/assets/${asset.id}`, '--header', 'Accept: application/octet-stream'));
      assert(raw.length === asset.size && raw.length <= 64 * 1024
        && asset.digest === `sha256:${digest(raw)}`, 'Legacy feed bytes differ from metadata');
      const current = JSON.parse(raw.toString('utf8')); validateLegacyManifest(current);
      const target = raw.equals(targetRaw[name]);
      if (!target) {
        validateChannelAdvance(current, legacy);
        assert(compareReleaseVersions(version, current.version) > 0, 'Same-version legacy bytes cannot be replaced by ordinary advancement');
      }
      return [name, { asset, target }];
    }));
  }
  function mirrorState(release) {
    const asset = release.assets.find(value => value.name === tar);
    assert(!asset || Number.isSafeInteger(asset.id) && asset.id > 0 && sameAsset(asset, mirror), 'Existing versioned legacy mirror differs; never overwrite it');
    return asset;
  }
  function write(name, file, clobber, previousAsset = null) {
    const current = fingerprint(file), wanted = name === tar ? mirror : { bytes: targetRaw[name].length, sha256: digest(targetRaw[name]) };
    assert(current.bytes === wanted.bytes && current.sha256 === wanted.sha256, 'Local migration payload changed');
    const action = { asset: name, release: 'update-channel', channelReleaseId: channelId,
      clobber, previousAsset: previousAsset ? { id: previousAsset.id, bytes: previousAsset.size, digest: previousAsset.digest } : null,
      targetBytes: current.bytes, targetSHA256: current.sha256 };
    writes.push(action); progress({ phase: 'before-mutation', action });
    try { gh('release', 'upload', 'update-channel', '--repo', repository, ...(clobber ? ['--clobber'] : []), file); }
    catch {
      const error = new Error(`Legacy advance upload outcome is unknown for ${name}; inspect the existing channel before an explicit new invocation`);
      error.code = 'LEGACY_ADVANCE_UNKNOWN_UPLOAD_OUTCOME';
      error.action = action; error.mutationMayHaveCompleted = true; error.automaticRetryAllowed = false;
      progress({ phase: 'unknown-mutation-outcome', action, automaticRetryAllowed: false });
      throw error;
    }
    progress({ phase: 'upload-returned', action });
  }
  const published = formal();
  for (const asset of published.assets) {
    const { destination } = download(asset, 'formal');
    if (asset.name === 'latest.json') assert(isDeepStrictEqual(JSON.parse(fs.readFileSync(destination, 'utf8')), manifest), 'Published modern manifest differs');
  }
  progress({ phase: 'six-formal-downloads-verified', assets: downloads });
  let current = channel(); feedStates(current);
  let existing = mirrorState(current);
  if (!existing) {
    formal(); current = channel(); feedStates(current); existing = mirrorState(current);
    if (!existing) write(tar, mirror.path, false);
  }
  current = channel(); feedStates(current); existing = mirrorState(current);
  assert(existing, 'Uploaded versioned legacy mirror missing; do not advance feeds');
  download(existing, 'mirror'); mirrorVerified = identity(existing);
  for (const name of feeds) {
    formal(); current = channel(); const states = feedStates(current);
    assert(isDeepStrictEqual(identity(mirrorState(current)), mirrorVerified), 'Verified legacy mirror changed');
    if (!states[name].target) { write(name, feedFiles[name], true, states[name].asset); changes.push(name); }
  }
  current = channel(); const final = feedStates(current);
  assert(Object.values(final).every(state => state.target)
    && isDeepStrictEqual(identity(mirrorState(current)), mirrorVerified), 'Legacy advancement incomplete; inspect existing state before resuming');
  const result = { schemaVersion: 1, kind: 'actual-higher-version-legacy-channel-advance', version, sourceCommit: commit,
    formalReleaseId: publishedId, channelReleaseId: channelId, formalAssetBytesVerified: 6,
    mirror: { name: tar, bytes: mirror.bytes, sha256: mirror.sha256 }, legacyFeedsUpdated: changes,
    mutationCalls: writes.length, formalReleaseModified: false, unrelatedChannelAssetsPreserved: true,
    originalSignaturesRetained: true, oldClientsCanUpgradeToNewerVersion: true, downloads };
  progress({ phase: 'complete', result });
  return result;
}
