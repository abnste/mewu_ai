// SPDX-License-Identifier: MPL-2.0
// Run only after the ordinary six-asset stable release is public and verified.
import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { repository, verifyRelease, validateUpdateConfiguration } from './release-contract.mjs';
import { legacyAdvanceManifest, publishLegacyAdvance } from './legacy-channel-advance.mjs';

if (process.argv.length !== 3) throw Error('Pass the accepted, already published newer stable assembly directory');
const directory = fs.realpathSync(process.argv[2]), metadata = path.join(directory, '.metadata');
const manifest = JSON.parse(fs.readFileSync(path.join(metadata, 'update.json'), 'utf8'));
const acceptance = JSON.parse(fs.readFileSync(path.join(metadata, 'acceptance.json'), 'utf8'));
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (commit !== acceptance.sourceCommit || execFileSync('git', ['status', '--porcelain', '-z']).length) throw Error('Exact clean accepted source required');
const config = JSON.parse(fs.readFileSync('apps/desktop/src-tauri/tauri.conf.json', 'utf8'));
if (config.version !== manifest.version) throw Error('Accepted assembly and source product versions differ');
validateUpdateConfiguration(config);
const legacy = legacyAdvanceManifest(manifest), release = verifyRelease(directory, manifest, acceptance, commit);
const root = path.join(fs.realpathSync(process.cwd()), '.private', 'release-migrations');
fs.mkdirSync(root, { recursive: true, mode: 0o700 });
if (fs.realpathSync(root) !== root || fs.lstatSync(root).isSymbolicLink()) throw Error('Private migration root must not be a link');
const work = path.join(root, `${manifest.version}-${randomUUID()}`);
fs.mkdirSync(work, { mode: 0o700 });
const downloads = path.join(work, 'verified-downloads'); fs.mkdirSync(downloads, { mode: 0o700 });
const feedFiles = Object.fromEntries(['preview.json', 'latest.json'].map(name => [name, path.join(work, name)]));
for (const file of Object.values(feedFiles)) fs.writeFileSync(file, JSON.stringify(legacy, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
const gh = (...args) => execFileSync('gh', args, { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] });
const events = path.join(work, 'events.jsonl');
const record = value => fs.appendFileSync(events, JSON.stringify({ observedAtUtc: new Date().toISOString(), ...value }) + '\n', { mode: 0o600 });
let attemptedUploads = 0, returnedUploads = 0;
const progress = value => {
  if (value.phase === 'before-mutation') attemptedUploads++;
  if (value.phase === 'upload-returned') returnedUploads++;
  record(value);
};
try {
  const result = publishLegacyAdvance(gh, repository, {
    manifest, commit, artifacts: release.files.map(file => ({ ...file, path: path.join(directory, file.name) })),
    feedFiles, workDirectory: downloads,
    verifyLocal: () => verifyRelease(directory, manifest, acceptance, commit), onProgress: progress,
    downloadAsset: (asset, destination) => {
      const fd = fs.openSync(destination, 'wx', 0o600);
      try { execFileSync('gh', ['api', `repos/${repository}/releases/assets/${asset.id}`, '--header', 'Accept: application/octet-stream'], { stdio: ['pipe', fd, 'pipe'] }); }
      finally { fs.closeSync(fd); }
    },
  });
  fs.writeFileSync(path.join(work, 'result.actual.json'), JSON.stringify(result, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
  console.log(JSON.stringify({ ...result, evidenceDirectory: work }));
} catch (error) {
  record({ phase: 'stopped', error: error.message, code: error.code ?? null,
    action: error.action ?? null, mutationMayHaveCompleted: error.mutationMayHaveCompleted ?? null,
    attemptedUploads, returnedUploads, automaticRetryAllowed: false });
  console.error(`Migration stopped; inspect preserved evidence at ${work}. No automatic retry.`);
  throw error;
}
