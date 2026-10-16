// SPDX-License-Identifier: MPL-2.0
// A numeric discovery alias for the already published preview installer.
// This never changes product versions, updater feeds, or the source preview.
import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { repository, releasePlan, validateManifest } from './release-contract.mjs';

const fail = message => { throw Error(message); };
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const commitPattern = /^[a-f0-9]{40}$/;
const digestPattern = /^sha256:[a-f0-9]{64}$/;
export const maximumInstallerBytes = 512 * 1024 * 1024;
export const canonicalAssetUrl = (tag, name) => `https://github.com/${repository}/releases/download/${tag}/${name}`;

// Match the installed 0.7.4 System.Version grammar, including its fourth field.
export function legacyVersion(tag) {
  if (typeof tag !== 'string' || !/^v(?:0|[1-9]\d*)(?:\.(?:0|[1-9]\d*)){2,3}$/.test(tag)) fail('Legacy numeric version required');
  const parts = tag.slice(1).split('.').map(Number);
  if (parts.some(value => !Number.isSafeInteger(value) || value > 2147483647)) fail('Legacy version component overflow');
  if (parts.length === 3) parts.push(-1);
  return parts;
}
export function compareLegacyVersions(a, b) {
  const left = legacyVersion(a), right = legacyVersion(b);
  for (let i = 0; i < 4; i++) if (left[i] !== right[i]) return left[i] < right[i] ? -1 : 1;
  return 0;
}
export function bridgePlan(request) {
  if (!request || request.schemaVersion !== 1 || !commitPattern.test(request.source?.commit) || !commitPattern.test(request.targetCommit) || !commitPattern.test(request.expectedMasterCommit)) fail('Pinned bridge source/target/master required');
  const match = /^1\.(0|[1-9]\d*)\.(0|[1-9]\d*)-preview\.(0|[1-9]\d*)$/.exec(request.source.version);
  if (!match) fail('Published 1.x preview source required');
  const version = `1.${match[1]}.${match[2]}.${match[3]}`, tag = `v${version}`;
  legacyVersion(tag);
  const latest = request.expectedLatest;
  if (!latest || !Number.isSafeInteger(latest.id) || latest.id < 1 || !commitPattern.test(latest.commit)) fail('Pinned current latest release required');
  if (legacyVersion(latest.tag)[0] >= 1 && latest.tag.split('.').length === 3) fail('A production stable release cannot be replaced by a preview bridge');
  if (compareLegacyVersions(tag, latest.tag) <= 0) fail('Refusing legacy bridge version downgrade or repeat');
  const installer = request.source.installer;
  if (!installer || !Number.isSafeInteger(installer.bytes) || installer.bytes < 1 || installer.bytes > maximumInstallerBytes || !/^[a-f0-9]{64}$/.test(installer.sha256)) fail('Pinned preview installer bytes/SHA256 required');
  if (request.notes !== undefined && (typeof request.notes !== 'string' || Buffer.byteLength(request.notes) > 16384 || request.notes.includes('\0'))) fail('Invalid additional bridge notes');
  const sourceTag = `v${request.source.version}`, sourceName = releasePlan(request.source.version)['windows-x86_64'].updater;
  const alias = `MewuAI-Remake-Setup-${version}-win-x64.exe`;
  return { repository, version, tag, alias, sourceTag, sourceName, sourceVersion: request.source.version,
    sourceCommit: request.source.commit, targetCommit: request.targetCommit, expectedMasterCommit: request.expectedMasterCommit,
    expectedLatest: { ...latest }, bytes: installer.bytes, sha256: installer.sha256,
    sourceUrl: canonicalAssetUrl(sourceTag, sourceName), aliasUrl: canonicalAssetUrl(tag, alias),
    // Existing major>=1 updater selects NSIS, never the old Inno argument list.
    installerKind: 'RemakeNsis', installerArguments: ['/S'],
    title: `旧版更新兼容入口 · MewuAI ${request.source.version}`,
    body: `供旧版 0.7.4 更新器使用的兼容入口。数字标签 ${tag} 仅用于旧更新器识别；实际安装的软件仍是 **${request.source.version}**，不是新的 ${version} 产品版本。\n\n` +
      `安装包与 [${request.source.version} 原发行版](https://github.com/${repository}/releases/tag/${sourceTag}) 完全相同，仅附件名适配旧更新器。产品源码提交：${request.source.commit}。本入口文档与发行脚本提交：${request.targetCommit}。\n\n` +
      `安装包 SHA-256：\`${installer.sha256}\`。安装后继续使用原预览更新通道。旧客户端会在下载和安装前分别请求确认；安装后如未自动启动，请从原快捷方式启动。\n\n` +
      `Compatibility entry for the 0.7.4 updater. The numeric tag ${tag} is a discovery alias; the actual installed version is ${request.source.version}, using the unchanged published installer bytes and product source ${request.source.commit}.\n` +
      (request.notes ? `\n${request.notes}\n` : '') };
}

// Complete Git diff admission, not an assertion that a new product was built.
export function validateBridgeChanges(changedPaths) {
  const scripts = new Set(['scripts/legacy-update-bridge.mjs', 'scripts/publish-legacy-update-bridge.mjs', 'scripts/legacy-update-bridge.test.mjs', 'scripts/release-staging.test.mjs']);
  const docs = new Set(['README.md', 'README.zh-CN.md', 'apps/desktop/README.md', 'docs/legacy-updates.md', 'docs/releasing.md']);
  if (!Array.isArray(changedPaths) || changedPaths.some(p => typeof p !== 'string' || (!scripts.has(p) && !docs.has(p)))) fail('Bridge target changes product/runtime or unapproved files');
  return true;
}
function assetIdentity(asset) {
  if (!asset || !Number.isSafeInteger(asset.id) || asset.id < 1 || !Number.isSafeInteger(asset.size) || asset.size < 1 || !digestPattern.test(asset.digest)) fail('Missing or invalid GitHub asset SHA256 metadata');
  return { id: asset.id, name: asset.name, size: asset.size, digest: asset.digest, browser_download_url: asset.browser_download_url };
}
function releaseIdentity(release) {
  if (!Number.isSafeInteger(release.id) || release.id < 1) fail('Invalid GitHub release ID');
  return { id: release.id, tag_name: release.tag_name, target_commitish: release.target_commitish,
    draft: release.draft, prerelease: release.prerelease,
    assets: release.assets.map(assetIdentity).sort((a, b) => a.name.localeCompare(b.name)) };
}
function transientDraftAssetUrl(value, name) {
  try {
    const temporaryTag = new URL(value).pathname.split('/').at(-2);
    return /^untagged-[a-f0-9]{20}$/.test(temporaryTag) && value === canonicalAssetUrl(temporaryTag, name);
  } catch { return false; }
}
function expectedAsset(asset, tag, name, bytes, checksum, draft = false) {
  const actual = assetIdentity(asset);
  const urlMatches = actual.browser_download_url === canonicalAssetUrl(tag, name) || (draft && transientDraftAssetUrl(actual.browser_download_url, name));
  if (actual.name !== name || actual.size !== bytes || actual.digest !== `sha256:${checksum}` || !urlMatches) fail('Exact bridge/source asset identity differs');
  return actual;
}
function verifyBuffer(buffer, plan) {
  if (!(buffer instanceof Uint8Array) || buffer.byteLength !== plan.bytes || sha(buffer) !== plan.sha256) fail('Downloaded installer bytes/SHA256 differ');
}
async function previewAndFeeds(github, download, plan) {
  const source = await github.release(plan.sourceTag);
  if (!source || source.draft !== false || source.prerelease !== true || source.tag_name !== plan.sourceTag || source.target_commitish !== plan.sourceCommit || await github.tagCommit(plan.sourceTag) !== plan.sourceCommit) fail('Published preview source/tag differs');
  const names = Object.values(releasePlan(plan.sourceVersion)).flatMap(entry => entry.files).sort();
  if (!Array.isArray(source.assets) || source.assets.map(a => a.name).sort().join() !== names.join()) fail('Preview must retain its four original public assets');
  for (const asset of source.assets) if (asset.browser_download_url !== canonicalAssetUrl(plan.sourceTag, asset.name)) fail('Noncanonical preview asset URL');
  const installer = source.assets.find(a => a.name === plan.sourceName);
  expectedAsset(installer, plan.sourceTag, plan.sourceName, plan.bytes, plan.sha256);
  const channel = await github.release('update-channel');
  if (!channel || channel.draft !== false || channel.prerelease !== true || !Array.isArray(channel.assets) || new Set(channel.assets.map(a => a.name)).size !== channel.assets.length) fail('Published update channel required');
  const feeds = channel.assets.filter(a => ['preview.json', 'latest.json'].includes(a.name));
  if (!feeds.some(a => a.name === 'preview.json')) fail('Existing preview feed required');
  const feedHashes = [];
  for (const asset of feeds) {
    assetIdentity(asset);
    if (asset.size > 65536 || asset.browser_download_url !== canonicalAssetUrl('update-channel', asset.name)) fail('Invalid protected feed');
    const bytes = await download(asset.browser_download_url, asset.size);
    if (!(bytes instanceof Uint8Array) || bytes.byteLength !== asset.size || `sha256:${sha(bytes)}` !== asset.digest) fail('Protected feed bytes differ');
    if (asset.name === 'preview.json' && validateManifest(JSON.parse(Buffer.from(bytes).toString('utf8'))) !== plan.sourceVersion) fail('Preview feed/source version differs');
    feedHashes.push({ name: asset.name, sha256: sha(bytes) });
  }
  if (await github.branchCommit('master') !== plan.expectedMasterCommit) fail('Protected master changed');
  return { source: releaseIdentity(source), channel: releaseIdentity(channel), feedHashes };
}
async function verifyBridge(github, release, plan, complete = false) {
  if (!release || !Number.isSafeInteger(release.id) || release.id < 1 || release.tag_name !== plan.tag || release.target_commitish !== plan.targetCommit || typeof release.draft !== 'boolean' || release.prerelease !== false || release.name !== plan.title || release.body !== plan.body || !Array.isArray(release.assets)) fail('Existing bridge release identity/body differs');
  const target = await github.tagCommit(plan.tag);
  if (target !== plan.targetCommit && !(target === null && release.draft)) fail('Bridge tag target differs');
  if (release.assets.length > 1 || (complete && release.assets.length !== 1)) fail('Bridge asset set differs; never overwrite a published release');
  if (release.assets.length) expectedAsset(release.assets[0], plan.tag, plan.alias, plan.bytes, plan.sha256, release.draft);
  if (!release.draft && release.assets.length !== 1) fail('Published bridge is incomplete');
}
async function verifyLatest(github, latest, bridge, plan) {
  if (!latest || latest.draft !== false || latest.prerelease !== false) fail('Invalid current latest release');
  if (bridge && !bridge.draft && latest.id === bridge.id && latest.tag_name === plan.tag) return 'bridge';
  if (latest.id !== plan.expectedLatest.id || latest.tag_name !== plan.expectedLatest.tag || await github.tagCommit(latest.tag_name) !== plan.expectedLatest.commit) fail('Current latest release differs from reviewed baseline');
  if (compareLegacyVersions(plan.tag, latest.tag_name) <= 0) fail('Refusing current latest downgrade');
  return 'baseline';
}

// Dependency injection keeps tests entirely offline. Only this bridge release
// may be created/uploaded/published; source releases, feeds and refs are read-only.
export async function publishLegacyBridge({ request, github, download, assertSource }) {
  const plan = bridgePlan(request);
  await assertSource(plan);
  const protectedBefore = await previewAndFeeds(github, download, plan);
  let bridge = await github.release(plan.tag);
  if (bridge) await verifyBridge(github, bridge, plan);
  await verifyLatest(github, await github.latest(), bridge, plan);
  const original = await download(plan.sourceUrl, plan.bytes);
  verifyBuffer(original, plan);
  const aliasBytes = Buffer.from(original); // Separate owned bytes; no executable changes.
  if (!bridge) {
    bridge = await github.createDraft({ tag_name: plan.tag, target_commitish: plan.targetCommit, name: plan.title, body: plan.body, draft: true, prerelease: false, make_latest: 'false' });
    await verifyBridge(github, bridge, plan);
  }
  if (!bridge.assets.length) {
    if (!bridge.draft) fail('Cannot upload into a published bridge');
    verifyBuffer(aliasBytes, plan);
    await github.upload(bridge.id, plan.tag, plan.alias, aliasBytes);
  }
  bridge = await github.release(plan.tag);
  await verifyBridge(github, bridge, plan, true);
  // Draft browser_download_url is not public. Authenticate the exact validated
  // asset before exposure; canonical anonymous download is checked afterwards.
  const wasDraft = bridge.draft;
  const publishedBytes = wasDraft ? await github.readBridgeAsset(bridge, plan.alias) : await download(plan.aliasUrl, plan.bytes);
  verifyBuffer(publishedBytes, plan);
  if (!Buffer.from(publishedBytes).equals(Buffer.from(original))) fail('Bridge alias is not byte-for-byte source installer');
  await assertSource(plan);
  if (!isDeepStrictEqual(await previewAndFeeds(github, download, plan), protectedBefore)) fail('Protected preview assets/feed/master changed');
  const latestState = await verifyLatest(github, await github.latest(), bridge, plan);
  if (bridge.draft || latestState !== 'bridge') {
    // This is the only operation that exposes the numeric entry to old clients.
    await github.publishLatest(bridge.id, { tag_name: plan.tag, target_commitish: plan.targetCommit, draft: false, prerelease: false, make_latest: 'true' });
  }
  bridge = await github.release(plan.tag);
  await verifyBridge(github, bridge, plan, true);
  if (bridge.draft || await verifyLatest(github, await github.latest(), bridge, plan) !== 'bridge') fail('Bridge was not made current latest');
  if (wasDraft) {
    const publicBytes = await download(plan.aliasUrl, plan.bytes);
    verifyBuffer(publicBytes, plan);
    if (!Buffer.from(publicBytes).equals(Buffer.from(original))) fail('Public bridge alias changed installer bytes');
  }
  if (!isDeepStrictEqual(await previewAndFeeds(github, download, plan), protectedBefore)) fail('Protected preview assets/feed/master changed after publication');
  return { tag: plan.tag, actualProductVersion: plan.sourceVersion, productSourceCommit: plan.sourceCommit, bridgeSourceCommit: plan.targetCommit,
    artifact: { name: plan.alias, bytes: plan.bytes, sha256: plan.sha256 }, originalBytesUnchanged: true, previewAndFeedsUnchanged: true,
    bridgeReleaseId: bridge.id, url: `https://github.com/${repository}/releases/tag/${plan.tag}` };
}
