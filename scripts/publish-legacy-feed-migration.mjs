// SPDX-License-Identifier: MPL-2.0
// Independent exact-authorized repair, never called by the normal publisher.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { execFileSync } from 'node:child_process';
import { repository, verifyRelease } from './release-contract.mjs';
import { legacyRepairManifest, publishLegacyRepair } from './legacy-feed-migration.mjs';
if (process.argv.length !== 4) throw Error('Pass the accepted published 1.0.0 assembly directory and exact repair authorization JSON');
const directory = fs.realpathSync(process.argv[2]), metadata = path.join(directory, '.metadata');
const read = name => JSON.parse(fs.readFileSync(path.join(metadata, name), 'utf8'));
const manifest = read('update.json'), acceptance = read('acceptance.json'), commit = acceptance.sourceCommit;
const repairAuthorization = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));
if (execFileSync('git', ['status', '--porcelain', '-z']).length) throw Error('Source must be clean');
const legacy = legacyRepairManifest(manifest), release = verifyRelease(directory, manifest, acceptance, commit);
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'mewu-legacy-feed-'));
const feedFiles = Object.fromEntries(['preview.json', 'latest.json'].map(name => [name, path.join(work, name)]));
try {
  for (const file of Object.values(feedFiles)) fs.writeFileSync(file, JSON.stringify(legacy, null, 2) + '\n', { flag: 'wx' });
  const gh = (...args) => execFileSync('gh', args, { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] });
  const result = publishLegacyRepair(gh, repository, { manifest, commit, feedFiles, repairAuthorization, artifacts: release.files.map(row => ({ ...row, path: path.join(directory, row.name) })), verifyLocal: () => verifyRelease(directory, manifest, acceptance, commit) });
  console.log(JSON.stringify(result));
} finally {
  for (const file of Object.values(feedFiles)) if (fs.existsSync(file)) fs.unlinkSync(file);
  fs.rmdirSync(work);
}
