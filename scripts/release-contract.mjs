// SPDX-License-Identifier: MPL-2.0
// One source/version, explicit assets, and native acceptance on all three OSes.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
export const repository = 'abnste/mewu_ai';
export const platforms = ['windows-x86_64', 'darwin-aarch64', 'linux-x86_64'];
export const requiredChecks = ['packageIntegrity', 'install', 'launch', 'restart', 'capture', 'drawing', 'objects', 'conversation', 'settings', 'recording', 'ocr', 'copySave', 'update'];
export function releaseVersion(version) {
  if (typeof version !== 'string' || version.length > 64 || !/^1\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-(?:preview|alpha|beta|rc)\.(?:0|[1-9]\d*))?$/.test(version)) throw Error('Invalid release version');
  return version;
}
// Installed previews keep their original fixed feed when moving to stable.
// Stable clients use latest.json; every stable publication advances both feeds.
export function publicationPolicy(version) {
  releaseVersion(version);
  const prerelease = version.includes('-');
  return {
    prerelease,
    latestArguments: prerelease ? ['--latest=false'] : ['--latest'],
    feeds: prerelease ? ['preview.json'] : ['preview.json', 'latest.json'],
    clientEndpoint: `https://github.com/${repository}/releases/download/update-channel/${prerelease ? 'preview.json' : 'latest.json'}`,
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
  return {
    'windows-x86_64': { files: [`MewuAI-Remake-Setup-${version}-win-x64.exe`, `MewuAI-Remake-Portable-${version}-win-x64.zip`], updater: `MewuAI-Remake-Setup-${version}-win-x64.exe`, channel: false },
    'darwin-aarch64': { files: [`MewuAI-Remake-${version}-mac-arm64.dmg`], updater: `MewuAI-Remake-Update-${version}-mac-arm64.app.tar.gz`, channel: true },
    'linux-x86_64': { files: [`MewuAI-Remake-${version}-linux-x64.AppImage`], updater: `MewuAI-Remake-${version}-linux-x64.AppImage`, channel: false },
  };
}
export function updateUrl(version, platform) {
  const entry = releasePlan(version)[platform];
  if (!entry) throw Error('Unsupported release platform');
  return `https://github.com/${repository}/releases/download/${entry.channel ? 'update-channel' : `v${version}`}/${entry.updater}`;
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
export function validateManifest(manifest) {
  const version = releaseVersion(manifest.version);
  if (!manifest.platforms || Object.keys(manifest.platforms).sort().join() !== [...platforms].sort().join()) throw Error('Three update platforms required');
  for (const platform of platforms) {
    const value = manifest.platforms[platform];
    if (!value || value.url !== updateUrl(version, platform)) throw Error('Invalid platform update URL');
    verifySignatureVersion(value.signature, version);
  }
  return version;
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
  const version = validateManifest(manifest), plan = releasePlan(version);
  const names = [...platforms.flatMap(platform => plan[platform].files), plan['darwin-aarch64'].updater];
  const entries = fs.readdirSync(directory, { withFileTypes: true });
  if (entries.length !== names.length + 1 || entries.some(entry => entry.name === '.metadata' ? !entry.isDirectory() || entry.isSymbolicLink() : !names.includes(entry.name) || !entry.isFile() || entry.isSymbolicLink())) throw Error('Unexpected release artifact');
  const metadata = path.join(directory, '.metadata');
  const metaEntries = fs.readdirSync(metadata, { withFileTypes: true });
  if (metaEntries.length !== 2 || metaEntries.some(entry => !['update.json', 'acceptance.json'].includes(entry.name) || !entry.isFile() || entry.isSymbolicLink())) throw Error('Unexpected release metadata');
  if (!acceptance || acceptance.sourceCommit !== commit || acceptance.version !== version || Object.keys(acceptance.platforms || {}).sort().join() !== [...platforms].sort().join()) throw Error('Missing three-platform acceptance');
  const files = new Map(names.map(name => [name, fingerprint(path.join(directory, name))]));
  for (const platform of platforms) {
    const assets = [...new Set([...plan[platform].files, plan[platform].updater])].map(name => files.get(name));
    validateAcceptance(acceptance.platforms[platform], platform, version, commit, assets);
  }
  return { version, tag: `v${version}`, assets: platforms.flatMap(platform => plan[platform].files), channelAssets: [plan['darwin-aarch64'].updater], files: [...files.values()] };
}
