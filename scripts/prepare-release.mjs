// SPDX-License-Identifier: MPL-2.0
// Stage one native platform. This command never publishes or copies keys.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { releaseVersion, releasePlan, updateUrl, fingerprint, regularFile, verifySignatureVersion } from './release-contract.mjs';
const root = fs.realpathSync(process.cwd());
const version = releaseVersion(JSON.parse(fs.readFileSync('apps/desktop/src-tauri/tauri.conf.json', 'utf8')).version), tag = `v${version}`;
const platform = process.argv[3] || `${process.platform === 'win32' ? 'windows' : process.platform}-${process.arch === 'arm64' ? 'aarch64' : process.arch === 'x64' ? 'x86_64' : process.arch}`;
const plan = releasePlan(version)[platform];
if (!plan) throw Error('Unsupported native release platform');
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (!/^[a-f0-9]{40}$/.test(commit) || execFileSync('git', ['status', '--porcelain', '-z']).length) throw Error('Build source must be clean and committed');
const target = path.resolve(process.env.CARGO_TARGET_DIR || 'target');
const output = path.resolve(process.argv[2] || `out/release-${platform}`);
const relative = path.relative(root, output);
if (!relative || relative.startsWith('..') || path.isAbsolute(relative) || fs.existsSync(output)) throw Error('Release staging must be new and inside workspace');
// Optional JSON request contains exact native paths, never shell fragments.
const request = process.argv[4] ? JSON.parse(fs.readFileSync(process.argv[4], 'utf8')) : {};
const defaults = {
  'windows-x86_64': { installer: path.join(target, `release/bundle/nsis/MewuAI_${version}_x64-setup.exe`), updater: null, binary: path.join(target, 'release/MewuAI.exe') },
  'darwin-aarch64': { installer: path.join(target, `release/bundle/dmg/MewuAI_${version}_aarch64.dmg`), updater: path.join(target, 'release/bundle/macos/MewuAI.app.tar.gz') },
  'linux-x86_64': { installer: path.join(target, `release/bundle/appimage/MewuAI_${version}_amd64.AppImage`), updater: null },
}[platform];
const installer = path.resolve(request.installer || defaults.installer), updater = path.resolve(request.updater || defaults.updater || installer);
regularFile(installer); regularFile(updater);
if (!plan.channel && updater !== installer) throw Error('Platform must update from its exact installer');
const sources = [[installer, plan.files[0]], ...(plan.channel ? [[updater, plan.updater]] : [])], before = sources.map(([source]) => fingerprint(source));
fs.mkdirSync(output, { recursive: true });
for (const [source, name] of sources) fs.copyFileSync(source, path.join(output, name));
if (platform === 'windows-x86_64') {
  const binary = path.resolve(request.binary || defaults.binary); regularFile(binary);
  const portable = path.join(output, '.portable'); fs.mkdirSync(portable);
  fs.copyFileSync(binary, path.join(portable, 'MewuAI.exe'));
  for (const [source, name] of [['LICENSE', 'LICENSE'], ['apps/desktop/THIRD-PARTY-NOTICES', 'THIRD-PARTY-NOTICES']]) fs.copyFileSync(source, path.join(portable, name));
  fs.writeFileSync(path.join(portable, 'SOURCE.md'), `# Mewu ${version}\n\nSource: https://github.com/abnste/mewu_ai/tree/${tag}\n\nCommit: ${commit}\n\nMPL-2.0. Third-party rights and license texts are retained in THIRD-PARTY-NOTICES.\n`);
  const quote = value => `'${value.replaceAll("'", "''")}'`;
  execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', `Compress-Archive -LiteralPath ${fs.readdirSync(portable).map(name => quote(path.join(portable, name))).join(',')} -DestinationPath ${quote(path.join(output, plan.files[1]))} -CompressionLevel Optimal`], { stdio: 'pipe' });
  for (const name of ['MewuAI.exe', 'LICENSE', 'THIRD-PARTY-NOTICES', 'SOURCE.md']) fs.unlinkSync(path.join(portable, name));
  fs.rmdirSync(portable);
}
for (let i = 0; i < sources.length; i++) {
  const current = fingerprint(sources[i][0]), copied = fingerprint(path.join(output, sources[i][1]));
  if ([current, copied].some(value => value.sha256 !== before[i].sha256 || value.bytes !== before[i].bytes)) throw Error('Build asset changed during staging');
}
const metadata = path.join(output, '.metadata'); fs.mkdirSync(metadata);
let signature = null;
if (fs.existsSync(`${updater}.sig`)) { regularFile(`${updater}.sig`); signature = fs.readFileSync(`${updater}.sig`, 'utf8').trim(); verifySignatureVersion(signature, version); }
const names = [...new Set([...plan.files, plan.updater])];
const build = { schemaVersion: 1, version, sourceCommit: commit, platform, assets: names.map(name => fingerprint(path.join(output, name))), updater: { target: platform, name: plan.updater, url: updateUrl(version, platform), signature } };
fs.writeFileSync(path.join(metadata, 'build.json'), JSON.stringify(build, null, 2) + '\n', { flag: 'wx' });
console.log(JSON.stringify({ version, sourceCommit: commit, platform, files: names, published: false }));
