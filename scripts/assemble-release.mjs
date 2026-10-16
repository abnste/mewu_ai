// SPDX-License-Identifier: MPL-2.0
// Combine all native packages only after matching installation acceptance.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { platforms, releaseVersion, releasePlan, updateUrl, fingerprint, verifySignatureVersion, validateAcceptance, verifyRelease, validateUpdateConfiguration, selfContainedRelease, githubReleaseUpdates } from './release-contract.mjs';
const root = fs.realpathSync(process.cwd());
if (!process.argv[2]) throw Error('A private three-platform assembly plan is required');
const request = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const config = JSON.parse(fs.readFileSync('apps/desktop/src-tauri/tauri.conf.json', 'utf8'));
validateUpdateConfiguration(config);
const version = releaseVersion(config.version), plan = releasePlan(version);
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (!/^[a-f0-9]{40}$/.test(commit) || execFileSync('git', ['status', '--porcelain', '-z']).length) throw Error('Source must be clean and committed');
for (const field of ['inputs', 'receipts']) if (Object.keys(request[field] || {}).sort().join() !== [...platforms].sort().join()) throw Error('Three platform inputs and receipts required');
const output = path.resolve(request.output || 'out/release'), relative = path.relative(root, output);
if (!relative || relative.startsWith('..') || path.isAbsolute(relative) || fs.existsSync(output)) throw Error('Assembly output must be new and inside workspace');
const manifest = { version, notes: `Mewu ${version}`, pub_date: new Date().toISOString(), platforms: {} }, acceptance = { schemaVersion: 1, sourceCommit: commit, version, platforms: {} }, transfers = [];
for (const platform of platforms) {
  const directory = fs.realpathSync(request.inputs[platform]);
  if (output === directory || output.startsWith(directory + path.sep) || directory.startsWith(output + path.sep)) throw Error('Input and output overlap');
  const entry = plan[platform], names = [...new Set([...entry.files, entry.updater])], entries = fs.readdirSync(directory, { withFileTypes: true });
  if (entries.length !== names.length + 1 || entries.some(e => e.name === '.metadata' ? !e.isDirectory() || e.isSymbolicLink() : !names.includes(e.name) || !e.isFile() || e.isSymbolicLink())) throw Error('Unexpected staged artifact');
  const meta = path.join(directory, '.metadata');
  if (fs.readdirSync(meta).length !== 1) throw Error('Unexpected staged metadata');
  const build = JSON.parse(fs.readFileSync(path.join(meta, 'build.json'), 'utf8'));
  if (build.schemaVersion !== 1 || build.version !== version || build.sourceCommit !== commit || build.platform !== platform || build.updater?.target !== platform || build.updater.name !== entry.updater || build.updater.url !== updateUrl(version, platform)) throw Error('Staged build identity mismatch');
  const assets = names.map(name => fingerprint(path.join(directory, name)));
  if (!Array.isArray(build.assets) || build.assets.length !== assets.length || assets.some(a => !build.assets.some(b => a.name === b.name && a.sha256 === b.sha256 && a.bytes === b.bytes))) throw Error('Staged artifact changed');
  const receipt = JSON.parse(fs.readFileSync(request.receipts[platform], 'utf8'));
  validateAcceptance(receipt, platform, version, commit, assets);
  let signature = build.updater.signature;
  if (request.signatures?.[platform]) signature = fs.readFileSync(request.signatures[platform], 'utf8').trim();
  verifySignatureVersion(signature, version);
  manifest.platforms[platform] = { signature, url: updateUrl(version, platform) };
  acceptance.platforms[platform] = Object.fromEntries(['schemaVersion', 'platform', 'version', 'sourceCommit', 'result', 'nativeGuiVerified', 'installerVerified', 'nativeTests', 'frontendTests', 'checks', 'artifacts'].map(key => [key, receipt[key]]));
  for (const asset of assets) transfers.push({ source: path.join(directory, asset.name), asset });
}
fs.mkdirSync(path.join(output, '.metadata'), { recursive: true });
for (const { source, asset } of transfers) {
  fs.copyFileSync(source, path.join(output, asset.name));
  const copied = fingerprint(path.join(output, asset.name)), current = fingerprint(source);
  if ([copied, current].some(value => value.sha256 !== asset.sha256 || value.bytes !== asset.bytes)) throw Error('Asset changed during assembly');
}
for (const [name, value] of [['update.json', manifest], ['acceptance.json', acceptance]]) fs.writeFileSync(path.join(output, '.metadata', name), JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
if (selfContainedRelease(version) && !githubReleaseUpdates(version)) fs.copyFileSync(path.join(output, '.metadata/update.json'), path.join(output, 'latest.json'), fs.constants.COPYFILE_EXCL);
const result = verifyRelease(output, manifest, acceptance, commit);
console.log(JSON.stringify({ version, sourceCommit: commit, assets: result.assets, channelAssets: result.channelAssets, published: false }));
