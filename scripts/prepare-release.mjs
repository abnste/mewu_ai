// SPDX-License-Identifier: MPL-2.0
// Only explicit distributable assets enter the release directory.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
const root = process.cwd();
const config = JSON.parse(fs.readFileSync('apps/desktop/src-tauri/tauri.conf.json', 'utf8'));
const version = config.version;
if (!/^1\.\d+\.\d+(?:-(?:preview|alpha|beta|rc)\.\d+)?$/.test(version)) throw Error('Invalid release version');
const tag = `v${version}`;
const target = path.resolve(process.env.CARGO_TARGET_DIR || 'target');
const bundle = path.join(target, 'release/bundle/nsis');
const expected = `MewuAI_${version}_x64-setup.exe`;
const installer = path.join(bundle, expected);
const binary = path.join(target, 'release/MewuAI.exe');
if (!fs.existsSync(installer) || !fs.existsSync(binary) || !fs.existsSync(`${installer}.sig`)) throw Error('Missing signed build');
const output = path.resolve(process.argv[2] || 'out/release');
if (!output.startsWith(root + path.sep)) throw Error('Release output must stay within this workspace');
if (fs.existsSync(output)) throw Error('Release directory must be new');
fs.mkdirSync(output, { recursive: true });
const metadata = path.join(output, '.metadata');
fs.mkdirSync(metadata);
const asset = `MewuAI-Remake-Setup-${version}-win-x64.exe`;
fs.copyFileSync(installer, path.join(output, asset));
const signature = fs.readFileSync(`${installer}.sig`, 'utf8').trim();
const signatureText = Buffer.from(signature, 'base64').toString('utf8');
const signedVersions = signatureText.split(/\r?\n/).filter(line => line.startsWith('trusted comment: '))
  .flatMap(line => line.slice('trusted comment: '.length).split('\t').filter(field => field.startsWith('version:')).map(field => field.slice('version:'.length)));
if (signedVersions.length !== 1 || signedVersions[0] !== version) throw Error('Signature does not bind the release version');
const manifest = { version, notes: `Mewu ${version}`, pub_date: new Date().toISOString(),
  platforms: { 'windows-x86_64': { signature, url: `https://github.com/abnste/mewu_ai/releases/download/${tag}/${asset}` } } };
fs.writeFileSync(path.join(metadata, 'update.json'), JSON.stringify(manifest, null, 2) + '\n');
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
// Portable distribution uses an allowlisted staging folder, never the workspace.
const portable = path.join(output, '.portable'); fs.mkdirSync(portable);
fs.copyFileSync(binary, path.join(portable, 'MewuAI.exe'));
for (const [source, dest] of [['LICENSE', 'LICENSE'], ['apps/desktop/THIRD-PARTY-NOTICES', 'THIRD-PARTY-NOTICES']]) fs.copyFileSync(source, path.join(portable, dest));
fs.writeFileSync(path.join(portable, 'SOURCE.md'), `# Mewu ${version}\n\nSource code: https://github.com/abnste/mewu_ai/tree/${tag}\n\nCommit: ${commit}\n\nMewu source is licensed under MPL-2.0. Third-party rights and license texts are retained in THIRD-PARTY-NOTICES.\n\nWindows x64 preview. Existing data is retained; legacy API connections are imported once. Legacy conversation history remains in the legacy data directory.\n`);
const zip = path.join(output, `MewuAI-Remake-Portable-${version}-win-x64.zip`);
const quote = value => `'${value.replaceAll("'", "''")}'`;
execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', `Compress-Archive -LiteralPath ${[...fs.readdirSync(portable)].map(name => quote(path.join(portable, name))).join(',')} -DestinationPath ${quote(zip)} -CompressionLevel Optimal`], { stdio: 'pipe' });
// Delete only these four known staging files and their now-empty directory.
for (const name of fs.readdirSync(portable)) fs.unlinkSync(path.join(portable, name));
fs.rmdirSync(portable);
// The signature is embedded in the update manifest; no separate signature or checksum file is distributed.
fs.unlinkSync(`${installer}.sig`);
console.log(JSON.stringify({ version, tag, commit, output, files: fs.readdirSync(output).filter(name => name !== '.metadata').sort() }, null, 2));
