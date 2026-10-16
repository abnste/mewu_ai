// SPDX-License-Identifier: MPL-2.0
// The version release and fixed update channel are advanced only after all
// native platform receipts match the exact packages and committed source.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { repository as repo, fingerprint, verifyRelease, validateChannelAdvance, validateUpdateConfiguration } from './release-contract.mjs';
import { findRelease, verifyReleaseTarget, verifyStableLatest } from './release-api.mjs';
const directory = path.resolve(process.argv[2] || 'out/release'), metadata = path.join(directory, '.metadata');
const manifest = JSON.parse(fs.readFileSync(path.join(metadata, 'update.json'), 'utf8'));
const acceptance = JSON.parse(fs.readFileSync(path.join(metadata, 'acceptance.json'), 'utf8'));
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (execFileSync('git', ['status', '--porcelain', '-z']).length) throw Error('Release source must be clean');
const config = JSON.parse(fs.readFileSync('apps/desktop/src-tauri/tauri.conf.json', 'utf8'));
if (config.version !== manifest.version) throw Error('Package/source version mismatch');
const policy = validateUpdateConfiguration(config);
// No GitHub command may occur before this gate, including draft creation.
const release = verifyRelease(directory, manifest, acceptance, commit), { tag, assets, channelAssets } = release;
const preview = policy.prerelease, notes = path.resolve(`docs/release-notes-${manifest.version}.md`);
if (!fs.lstatSync(notes).isFile() || fs.lstatSync(notes).isSymbolicLink()) throw Error('Missing release notes');
const gh = (...args) => execFileSync('gh', args, { stdio: ['pipe', 'pipe', 'pipe'], encoding: 'utf8' });
const lookup = name => findRelease(gh, repo, name);
const expected = new Map(release.files.map(file => [file.name, file]));
function checkAsset(asset) {
  const file = expected.get(asset.name);
  if (!file || asset.size !== file.bytes || asset.digest !== `sha256:${file.sha256}`) throw Error('Remote release asset differs; never replace a versioned package');
}
function uploadMissing(name, current, names) {
  for (const asset of current.assets) if (names.includes(asset.name)) checkAsset(asset);
  for (const file of names) if (!current.assets.some(asset => asset.name === file)) {
    const actual = fingerprint(path.join(directory, file)), original = expected.get(file);
    if (actual.sha256 !== original.sha256 || actual.bytes !== original.bytes) throw Error('Local release asset changed');
    gh('release', 'upload', name, '--repo', repo, path.join(directory, file));
  }
  const uploaded = lookup(name);
  for (const file of names) { const asset = uploaded.assets.find(asset => asset.name === file); if (!asset) throw Error('Missing uploaded package'); checkAsset(asset); }
  return uploaded;
}
const channels = policy.feeds;
let channelRelease = lookup('update-channel');
function checkChannels() {
  if (!channelRelease) return;
  if (channelRelease.draft || !channelRelease.prerelease) throw Error('Invalid update channel release');
  for (const name of channels) {
    const asset = channelRelease.assets.find(asset => asset.name === name);
    if (!asset) continue;
    if (asset.size > 64 * 1024) throw Error('Oversized update feed');
    const bytes = gh('api', `repos/${repo}/releases/assets/${asset.id}`, '--header', 'Accept: application/octet-stream');
    if (Buffer.byteLength(bytes) > 64 * 1024) throw Error('Oversized update feed');
    validateChannelAdvance(JSON.parse(bytes), manifest);
  }
}
checkChannels();
let existing = lookup(tag), alreadyPublished = false;
if (existing) {
  verifyReleaseTarget(gh, repo, existing, tag, commit, preview);
  if (existing.assets.some(asset => !assets.includes(asset.name))) throw Error('Existing release identity/assets differ');
  for (const asset of existing.assets) checkAsset(asset);
  if (!existing.draft) {
    if (existing.assets.length !== assets.length) throw Error('Published release incomplete; never mutate published packages');
    alreadyPublished = true;
  }
} else {
  gh('release', 'create', tag, '--repo', repo, '--target', commit, '--draft', ...(preview ? ['--prerelease'] : []), '--title', `MewuAI ${manifest.version} · 喵呜AI ${manifest.version}`, '--notes-file', notes);
  existing = lookup(tag);
  verifyReleaseTarget(gh, repo, existing, tag, commit, preview);
}
if (!alreadyPublished) uploadMissing(tag, existing, assets);
if (!channelRelease) {
  gh('release', 'create', 'update-channel', '--repo', repo, '--target', commit, '--prerelease', '--latest=false', '--title', 'MewuAI 自动更新 / Update channel', '--notes', '软件自动更新入口。下载软件请前往版本发行页。 / Automatic update feed.');
  channelRelease = lookup('update-channel');
}
if (channelRelease.draft || !channelRelease.prerelease) throw Error('Invalid update channel release');
uploadMissing('update-channel', channelRelease, channelAssets);
// Recheck local acceptance and every byte immediately before making a release visible.
verifyRelease(directory, manifest, acceptance, commit);
if (!alreadyPublished) gh('release', 'edit', tag, '--repo', repo, '--draft=false', ...policy.latestArguments);
if (!preview) {
  const published = lookup(tag);
  verifyReleaseTarget(gh, repo, published, tag, commit, false);
  await verifyStableLatest(gh, repo, published);
}
// Only the small fixed JSON feeds are replaceable. Versioned packages never are.
// Re-read just before advancing feeds to detect a concurrently published version.
channelRelease = lookup('update-channel'); checkChannels();
for (const channel of channels) {
  const file = path.join(metadata, channel);
  fs.writeFileSync(file, JSON.stringify(manifest, null, 2) + '\n', { flag: 'wx' });
  try { gh('release', 'upload', 'update-channel', '--repo', repo, '--clobber', file); }
  finally { fs.unlinkSync(file); }
}
console.log(`Published https://github.com/${repo}/releases/tag/${tag}`);
