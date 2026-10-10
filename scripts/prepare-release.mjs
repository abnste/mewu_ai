// SPDX-License-Identifier: MPL-2.0
// Only explicit distributable assets enter the release directory.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
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
const asset = `MewuAI-Remake-Setup-${version}-win-x64.exe`;
fs.copyFileSync(installer, path.join(output, asset));
fs.copyFileSync(`${installer}.sig`, path.join(output, `${asset}.sig`));
const signature = fs.readFileSync(`${installer}.sig`, 'utf8').trim();
const signatureText = Buffer.from(signature, 'base64').toString('utf8');
if (!signatureText.split(/\r?\n/).some(line => line.startsWith('trusted comment:') && line.includes(`\tversion:${version}`))) throw Error('Signature does not bind the release version');
const manifest = { version, notes: `Mewu ${version}`, pub_date: new Date().toISOString(),
  platforms: { 'windows-x86_64': { signature, url: `https://github.com/abnste/mewu_ai/releases/download/${tag}/${asset}` } } };
fs.writeFileSync(path.join(output, 'update.json'), JSON.stringify(manifest, null, 2) + '\n');
for (const [source, dest] of [['LICENSE', 'LICENSE'], ['apps/desktop/THIRD-PARTY-NOTICES', 'THIRD-PARTY-NOTICES']]) fs.copyFileSync(source, path.join(output, dest));
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
fs.writeFileSync(path.join(output, 'SOURCE.md'), `# Mewu ${version}\n\nSource code: https://github.com/abnste/mewu_ai/tree/${tag}\n\nCommit: ${commit}\n\nMewu source is licensed under MPL-2.0. Third-party rights and license texts are retained in THIRD-PARTY-NOTICES.\n\nWindows x64 preview. Existing data is retained; legacy API connections are imported once. Legacy conversation history remains in the legacy data directory.\n`);
// Portable distribution uses an allowlisted staging folder, never the workspace.
const portable = path.join(output, '.portable'); fs.mkdirSync(portable);
fs.copyFileSync(binary, path.join(portable, 'MewuAI.exe'));
for (const name of ['LICENSE', 'THIRD-PARTY-NOTICES', 'SOURCE.md']) fs.copyFileSync(path.join(output, name), path.join(portable, name));
const zip = path.join(output, `MewuAI-Remake-Portable-${version}-win-x64.zip`);
const quote = value => `'${value.replaceAll("'", "''")}'`;
execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', `Compress-Archive -LiteralPath ${[...fs.readdirSync(portable)].map(name => quote(path.join(portable, name))).join(',')} -DestinationPath ${quote(zip)} -CompressionLevel Optimal`], { stdio: 'pipe' });
// Delete only these four known staging files and their now-empty directory.
for (const name of fs.readdirSync(portable)) fs.unlinkSync(path.join(portable, name));
fs.rmdirSync(portable);
const names = fs.readdirSync(output).sort();
const sums = names.map(name => `${createHash('sha256').update(fs.readFileSync(path.join(output, name))).digest('hex')}  ${name}`).join('\n') + '\n';
fs.writeFileSync(path.join(output, 'SHA256SUMS.txt'), sums);
console.log(JSON.stringify({ version, tag, commit, output, files: fs.readdirSync(output).sort() }, null, 2));
