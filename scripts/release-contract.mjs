// SPDX-License-Identifier: MPL-2.0
// One source/version, explicit assets, and native acceptance on all three OSes.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
export const repository = 'abnste/mewu_ai';
export const platforms = ['windows-x86_64', 'darwin-aarch64', 'linux-x86_64'];
export const requiredChecks = ['packageIntegrity', 'install', 'launch', 'restart', 'capture', 'drawing', 'objects', 'conversation', 'settings', 'recording', 'ocr', 'copySave', 'update'];
export function releaseVersion(version) {
  if (typeof version !== 'string' || version.length > 64 || !/^1\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-(?:preview|alpha|beta|rc)\.(?:0|[1-9]\d*))?$/.test(version)) throw Error('Invalid release version');
  return version;
}
// Stable releases are self-contained. Historical 1.0.0 prereleases retain their
// original layout; ordinary publication never replaces published packages.
export function selfContainedRelease(version) {
  releaseVersion(version);
  const [, minor, patch] = /^1\.(\d+)\.(\d+)/.exec(version);
  return !version.includes('-') || BigInt(minor) > 0n || BigInt(patch) >= 1n;
}
// New clients discover the actual GitHub release. The old layouts remain
// readable for historical verification; no new release uploads a feed file.
export function githubReleaseUpdates(version) {
  releaseVersion(version);
  const [, minor, patch] = /^1\.(\d+)\.(\d+)/.exec(version);
  return BigInt(minor) > 0n || BigInt(patch) >= 1n;
}
export function publicationPolicy(version) {
  releaseVersion(version);
  const prerelease = version.includes('-'), selfContained = selfContainedRelease(version);
  return {
    prerelease,
    selfContained,
    latestArguments: prerelease ? ['--latest=false'] : ['--latest'],
    feeds: selfContained ? [] : prerelease ? ['preview.json'] : ['preview.json', 'latest.json'],
    // Modern previews also track the next stable release; publishing one does
    // not promote GitHub Latest or overwrite the old clients' migration feed.
    clientEndpoint: githubReleaseUpdates(version) ? `https://api.github.com/repos/${repository}/releases/latest` : selfContained ? `https://github.com/${repository}/releases/latest/download/latest.json` : `https://github.com/${repository}/releases/download/update-channel/${prerelease ? 'preview.json' : 'latest.json'}`,
  };
}
export function validateUpdateConfiguration(config) {
  const policy = publicationPolicy(config?.version), updater = config?.plugins?.updater;
  if (updater?.requireSignedVersion !== true || !Array.isArray(updater.endpoints) || updater.endpoints.length !== 1 || updater.endpoints[0] !== policy.clientEndpoint) throw Error('Source updater must use the fixed signed release channel for its actual product version');
  return policy;
}
// SemVer precedence for the intentionally restricted public release grammar.
export function compareReleaseVersions(left, right) {
  const parse = value => { releaseVersion(value); return /^1\.(\d+)\.(\d+)(?:-(preview|alpha|beta|rc)\.(\d+))?$/.exec(value); };
  const a = parse(left), b = parse(right), cmp = (x, y) => x < y ? -1 : x > y ? 1 : 0;
  for (const i of [1, 2]) { const result = cmp(BigInt(a[i]), BigInt(b[i])); if (result) return result; }
  if (!a[3] || !b[3]) return cmp(Boolean(!a[3]), Boolean(!b[3]));
  return cmp(a[3], b[3]) || cmp(BigInt(a[4]), BigInt(b[4]));
}
export function validateChannelAdvance(current, candidate) {
  const order = compareReleaseVersions(candidate.version, current.version);
  if (order < 0) throw Error('Refusing update channel downgrade');
  if (order === 0 && (Object.keys(current.platforms || {}).sort().join() !== Object.keys(candidate.platforms || {}).sort().join() || platforms.some(platform => current.platforms?.[platform]?.url !== candidate.platforms[platform]?.url || current.platforms?.[platform]?.signature !== candidate.platforms[platform]?.signature))) throw Error('Same version update bytes/targets differ');
}
export function releasePlan(version) {
  releaseVersion(version);
  const product = githubReleaseUpdates(version) ? 'MewuAI' : 'MewuAI-Remake';
  return {
    'windows-x86_64': { files: [`${product}-Setup-${version}-win-x64.exe`, `${product}-Portable-${version}-win-x64.zip`], updater: `${product}-Setup-${version}-win-x64.exe`, channel: false, separateUpdater: false },
    'darwin-aarch64': { files: [`${product}-${version}-mac-arm64.dmg`], updater: `${product}-Update-${version}-mac-arm64.app.tar.gz`, channel: !selfContainedRelease(version), separateUpdater: true },
    'linux-x86_64': { files: [`${product}-${version}-linux-x64.AppImage`], updater: `${product}-${version}-linux-x64.AppImage`, channel: false, separateUpdater: false },
  };
}
export function updateUrl(version, platform) {
  const entry = releasePlan(version)[platform];
  if (!entry) throw Error('Unsupported release platform');
  return `https://github.com/${repository}/releases/download/${entry.channel ? 'update-channel' : `v${version}`}/${entry.updater}`;
}
// Explicit old-client contract, independent of the new release's own layout.
// Old native Mac validators require this path even for the 1.0.0 repair bridge.
export function legacyUpdateUrl(version, platform) {
  const entry = releasePlan(version)[platform];
  if (!entry) throw Error('Unsupported release platform');
  return `https://github.com/${repository}/releases/download/${platform === 'darwin-aarch64' ? 'update-channel' : `v${version}`}/${entry.updater}`;
}
export function regularFile(file) {
  const stat = fs.lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size === 0) throw Error('Invalid release file');
  return stat;
}
export function fingerprint(file) {
  const before = regularFile(file), hash = createHash('sha256');
  const fd = fs.openSync(file, 'r');
  try {
    const opened = fs.fstatSync(fd);
    if (opened.dev !== before.dev || opened.ino !== before.ino) throw Error('Release file replaced');
    const buffer = Buffer.alloc(1024 * 1024);
    for (let count; (count = fs.readSync(fd, buffer, 0, buffer.length, null)) > 0;) hash.update(buffer.subarray(0, count));
    const after = fs.fstatSync(fd), named = regularFile(file);
    for (const stat of [after, named]) if (stat.size !== before.size || stat.mtimeMs !== before.mtimeMs || stat.ctimeMs !== before.ctimeMs || stat.ino !== before.ino || stat.dev !== before.dev) throw Error('Release file changed');
  } finally { fs.closeSync(fd); }
  return { name: path.basename(file), bytes: before.size, sha256: hash.digest('hex') };
}
export function verifySignatureVersion(signature, version) {
  if (typeof signature !== 'string' || !/^[A-Za-z0-9+/]+={0,2}$/.test(signature) || signature.length > 8192) throw Error('Invalid update signature');
  const bytes = Buffer.from(signature, 'base64');
  if (bytes.toString('base64') !== signature) throw Error('Invalid signature encoding');
  const lines = bytes.toString('utf8').split(/\r?\n/);
  const versions = lines.filter(line => line.startsWith('trusted comment: ')).flatMap(line => line.slice(17).split('\t').filter(field => field.startsWith('version:')).map(field => field.slice(8)));
  if (versions.length !== 1 || versions[0] !== version) throw Error('Signature does not bind release version');
  // Cryptographic byte verification is a separate packageIntegrity acceptance check.
}
function validateManifestUrls(manifest, urlFor) {
  const version = releaseVersion(manifest.version);
  if (!manifest.platforms || Object.keys(manifest.platforms).sort().join() !== [...platforms].sort().join()) throw Error('Three update platforms required');
  for (const platform of platforms) {
    const value = manifest.platforms[platform];
    if (!value || value.url !== urlFor(version, platform)) throw Error('Invalid platform update URL');
    verifySignatureVersion(value.signature, version);
  }
  return version;
}
export function validateManifest(manifest) { return validateManifestUrls(manifest, updateUrl); }
export function validateLegacyManifest(manifest) { return validateManifestUrls(manifest, legacyUpdateUrl); }
export const releaseSignatureMarker = '<!-- mewu-update-signatures:v1\n';
export function releaseSignatures(manifest) {
  const version = validateManifest(manifest), plan = releasePlan(version);
  return { schemaVersion: 1, version, assets: platforms.map(platform => ({ name: plan[platform].updater, signature: manifest.platforms[platform].signature })) };
}
export function composeReleaseNotes(notes, manifest) {
  if (typeof notes !== 'string' || Buffer.byteLength(notes) > 192 * 1024 || notes.includes('<!-- mewu-update-signatures:')) throw Error('Invalid original release notes');
  return `${notes.trimEnd()}\n\n${releaseSignatureMarker}${JSON.stringify(releaseSignatures(manifest))}\n-->\n`;
}
export function validateReleaseNotes(notes, version, manifest) {
  if (typeof notes !== 'string' || Buffer.byteLength(notes) > 256 * 1024 || notes.split('<!-- mewu-update-signatures:').length !== 2) throw Error('One release signature block required');
  const start = notes.indexOf(releaseSignatureMarker), end = start < 0 ? -1 : notes.indexOf('\n-->', start + releaseSignatureMarker.length);
  if (start < 0 || end < 0) throw Error('Invalid release signature block');
  let value;
  try { value = JSON.parse(notes.slice(start + releaseSignatureMarker.length, end)); } catch { throw Error('Invalid release signature JSON'); }
  const plan = releasePlan(version), names = platforms.map(platform => plan[platform].updater);
  if (!value || Object.keys(value).sort().join() !== 'assets,schemaVersion,version' || value.schemaVersion !== 1 || value.version !== version || !Array.isArray(value.assets) || value.assets.length !== names.length) throw Error('Release signature identity differs');
  const seen = new Set();
  for (const asset of value.assets) {
    if (!asset || Object.keys(asset).sort().join() !== 'name,signature' || !names.includes(asset.name) || seen.has(asset.name)) throw Error('Release signature assets differ');
    seen.add(asset.name); verifySignatureVersion(asset.signature, version);
  }
  if (manifest && !isDeepStrictEqual(value, releaseSignatures(manifest))) throw Error('Release signatures differ from accepted payloads');
  return value;
}
export function validateAcceptance(receipt, platform, version, commit, assets) {
  if (!receipt || receipt.schemaVersion !== 1 || receipt.platform !== platform || receipt.version !== version || receipt.sourceCommit !== commit || !/^[a-f0-9]{40}$/.test(commit)) throw Error('Acceptance source/version/platform mismatch');
  if (receipt.result !== 'passed' || receipt.nativeGuiVerified !== true || receipt.installerVerified !== true) throw Error('Native installation acceptance required');
  for (const suite of ['nativeTests', 'frontendTests']) {
    const value = receipt[suite];
    if (!value || !Number.isSafeInteger(value.passed) || value.passed < 1 || value.failed !== 0) throw Error('Failed or missing regression suite');
  }
  if (!receipt.checks || requiredChecks.some(check => receipt.checks[check] !== 'passed')) throw Error('Incomplete platform acceptance');
  if (!Array.isArray(receipt.artifacts) || receipt.artifacts.length !== assets.length) throw Error('Acceptance artifact set mismatch');
  const names = new Set();
  for (const expected of assets) {
    const actual = receipt.artifacts.find(value => value.name === expected.name);
    if (!actual || names.has(actual.name) || actual.sha256 !== expected.sha256 || actual.bytes !== expected.bytes) throw Error('Acceptance artifact fingerprint mismatch');
    names.add(actual.name);
  }
}
export function verifyRelease(directory, manifest, acceptance, commit) {
  const version = validateManifest(manifest), plan = releasePlan(version), modern = selfContainedRelease(version);
  const softwareAssets = platforms.flatMap(platform => plan[platform].files);
  const payloadNames = [...softwareAssets, plan['darwin-aarch64'].updater];
  const names = [...payloadNames, ...(modern && !githubReleaseUpdates(version) ? ['latest.json'] : [])];
  const entries = fs.readdirSync(directory, { withFileTypes: true });
  if (entries.length !== names.length + 1 || entries.some(entry => entry.name === '.metadata' ? !entry.isDirectory() || entry.isSymbolicLink() : !names.includes(entry.name) || !entry.isFile() || entry.isSymbolicLink())) throw Error('Unexpected release artifact');
  const metadata = path.join(directory, '.metadata');
  const metaEntries = fs.readdirSync(metadata, { withFileTypes: true });
  if (metaEntries.length !== 2 || metaEntries.some(entry => !['update.json', 'acceptance.json'].includes(entry.name) || !entry.isFile() || entry.isSymbolicLink())) throw Error('Unexpected release metadata');
  if (!acceptance || acceptance.sourceCommit !== commit || acceptance.version !== version || Object.keys(acceptance.platforms || {}).sort().join() !== [...platforms].sort().join()) throw Error('Missing three-platform acceptance');
  if (modern && !githubReleaseUpdates(version)) {
    const raw = fs.readFileSync(path.join(directory, 'latest.json'));
    if (raw.length > 64 * 1024 || !isDeepStrictEqual(JSON.parse(raw.toString('utf8')), manifest) || !raw.equals(fs.readFileSync(path.join(metadata, 'update.json')))) throw Error('Public latest.json differs from the accepted signed manifest');
  }
  const files = new Map(names.map(name => [name, fingerprint(path.join(directory, name))]));
  for (const platform of platforms) {
    const assets = [...new Set([...plan[platform].files, plan[platform].updater])].map(name => files.get(name));
    validateAcceptance(acceptance.platforms[platform], platform, version, commit, assets);
  }
  return { version, tag: `v${version}`, softwareAssets, assets: modern ? names : softwareAssets, channelAssets: modern ? [] : [plan['darwin-aarch64'].updater], files: [...files.values()] };
}
