// SPDX-License-Identifier: MPL-2.0
// Branch names never enter update URLs. A stable release also advances previews.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
const repo = 'abnste/mewu_ai';
const directory = path.resolve(process.argv[2] || 'out/release');
const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'update.json'), 'utf8'));
const tag = `v${manifest.version}`, preview = manifest.version.includes('-');
const gh = (...args) => execFileSync('gh', args, { stdio: ['pipe', 'pipe', 'pipe'], encoding: 'utf8' });
const approved = /^(?:MewuAI-Remake-(?:Setup|Portable)-[\w.-]+-win-x64\.(?:exe(?:\.sig)?|zip)|update\.json|SOURCE\.md|LICENSE|THIRD-PARTY-NOTICES|SHA256SUMS\.txt)$/;
const assets = fs.readdirSync(directory);
if (assets.some(name => !approved.test(name))) throw Error('Unexpected release artifact');
const notes = path.join(directory, 'SOURCE.md');
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
gh('release', 'create', tag, '--repo', repo, '--target', commit, '--draft', ...(preview ? ['--prerelease'] : []), '--title', `Mewu ${manifest.version}`, '--notes-file', notes);
gh('release', 'upload', tag, '--repo', repo, ...assets.map(name => path.join(directory, name)));
gh('release', 'edit', tag, '--repo', repo, '--draft=false', ...(preview ? ['--latest=false'] : ['--latest']));
// The channel release is always a prerelease so the legacy stable updater ignores it.
try { gh('release', 'view', 'update-channel', '--repo', repo); }
catch { gh('release', 'create', 'update-channel', '--repo', repo, '--target', commit, '--prerelease', '--latest=false', '--title', 'Mewu update channel', '--notes', 'Signed installer manifests. Versioned downloads are listed in Releases.'); }
for (const channel of preview ? ['preview.json'] : ['preview.json', 'latest.json']) {
  const file = path.join(directory, channel);
  fs.writeFileSync(file, JSON.stringify(manifest, null, 2) + '\n');
  gh('release', 'upload', 'update-channel', '--repo', repo, '--clobber', file);
  fs.unlinkSync(file);
}
console.log(`Published https://github.com/${repo}/releases/tag/${tag}`);
