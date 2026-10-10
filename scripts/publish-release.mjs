// SPDX-License-Identifier: MPL-2.0
// Branch names never enter update URLs. A stable release also advances previews.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
const repo = 'abnste/mewu_ai';
const directory = path.resolve(process.argv[2] || 'out/release');
const metadata = path.join(directory, '.metadata');
const manifest = JSON.parse(fs.readFileSync(path.join(metadata, 'update.json'), 'utf8'));
if (!/^1\.\d+\.\d+(?:-(?:preview|alpha|beta|rc)\.\d+)?$/.test(manifest.version)) throw Error('Invalid release version');
const tag = `v${manifest.version}`, preview = manifest.version.includes('-');
const gh = (...args) => execFileSync('gh', args, { stdio: ['pipe', 'pipe', 'pipe'], encoding: 'utf8' });
const assets = [`MewuAI-Remake-Setup-${manifest.version}-win-x64.exe`, `MewuAI-Remake-Portable-${manifest.version}-win-x64.zip`];
const expected = new Set([...assets, '.metadata']);
const entries = fs.readdirSync(directory, { withFileTypes: true });
if (entries.length !== expected.size || entries.some(entry => !expected.has(entry.name) || (entry.name === '.metadata' ? !entry.isDirectory() : !entry.isFile()))) throw Error('Unexpected release artifact');
if (fs.readdirSync(metadata).length !== 1 || !fs.statSync(path.join(metadata, 'update.json')).isFile()) throw Error('Unexpected release metadata');
const notes = path.resolve(`docs/release-notes-${manifest.version}.md`);
if (!fs.statSync(notes).isFile()) throw Error('Missing release notes');
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
let existing;
try { existing = JSON.parse(gh('api', `repos/${repo}/releases/tags/${tag}`)); }
catch (error) { if (!String(error.stderr).includes('404')) throw error; }
if (existing) {
  const ref = JSON.parse(gh('api', `repos/${repo}/git/ref/tags/${tag}`));
  if (ref.object.type !== 'commit' || ref.object.sha !== commit || existing.draft) throw Error('Existing version differs; never replace a published installer');
  console.log(`Version already published: https://github.com/${repo}/releases/tag/${tag}`);
  process.exit(0);
}
gh('release', 'create', tag, '--repo', repo, '--target', commit, '--draft', ...(preview ? ['--prerelease'] : []), '--title', `MewuAI ${manifest.version} · 喵呜AI ${manifest.version}`, '--notes-file', notes);
gh('release', 'upload', tag, '--repo', repo, ...assets.map(name => path.join(directory, name)));
gh('release', 'edit', tag, '--repo', repo, '--draft=false', ...(preview ? ['--latest=false'] : ['--latest']));
// The channel release is always a prerelease so the legacy stable updater ignores it.
try { gh('release', 'view', 'update-channel', '--repo', repo); }
catch { gh('release', 'create', 'update-channel', '--repo', repo, '--target', commit, '--prerelease', '--latest=false', '--title', 'MewuAI 自动更新 / Update channel', '--notes', '软件自动更新入口。下载软件请前往版本发行页。 / Automatic update feed. Download the app from a versioned release.'); }
for (const channel of preview ? ['preview.json'] : ['preview.json', 'latest.json']) {
  const file = path.join(metadata, channel);
  fs.writeFileSync(file, JSON.stringify(manifest, null, 2) + '\n');
  gh('release', 'upload', 'update-channel', '--repo', repo, '--clobber', file);
  fs.unlinkSync(file);
}
console.log(`Published https://github.com/${repo}/releases/tag/${tag}`);
