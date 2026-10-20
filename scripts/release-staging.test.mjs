// SPDX-License-Identifier: MPL-2.0
// Executes staging/assembly in an isolated fixture Git repository. Dummy bytes
// are never installers, and successful fixtures never invoke the publisher.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { platforms, requiredChecks, fingerprint, releasePlan, updateUrl, verifyRelease, publicationPolicy, selfContainedRelease, githubReleaseUpdates } from './release-contract.mjs';
const baselineVersion = '1.0.0-preview.3', scripts = path.dirname(fileURLToPath(import.meta.url));
function fixture(run, version = baselineVersion) {
  const parent = fs.mkdtempSync(path.join(os.tmpdir(), 'mewu-staging-')), root = path.join(parent, 'repo');
  fs.mkdirSync(path.join(root, 'scripts'), { recursive: true });
  try {
    for (const name of ['release-contract.mjs', 'release-api.mjs', 'cli-package.mjs', 'prepare-release.mjs', 'assemble-release.mjs', 'publish-release.mjs', 'legacy-update-bridge.mjs', 'publish-legacy-update-bridge.mjs', 'legacy-feed-migration.mjs', 'publish-legacy-feed-migration.mjs']) fs.copyFileSync(path.join(scripts, name), path.join(root, 'scripts', name));
    fs.mkdirSync(path.join(root, 'apps/desktop/src-tauri'), { recursive: true });
    fs.mkdirSync(path.join(root, 'docs'));
    fs.writeFileSync(path.join(root, '.gitignore'), 'out/\n');
    fs.writeFileSync(path.join(root, 'apps/desktop/src-tauri/tauri.conf.json'), JSON.stringify({ version, plugins: { updater: { endpoints: [publicationPolicy(version).clientEndpoint], requireSignedVersion: true } } }));
    fs.writeFileSync(path.join(root, 'LICENSE'), 'fixture license');
    fs.writeFileSync(path.join(root, 'apps/desktop/THIRD-PARTY-NOTICES'), 'fixture notices');
    fs.writeFileSync(path.join(root, `docs/release-notes-${version}.md`), '# Test fixture, never publish\n');
    const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    git('init', '--quiet'); git('add', '.');
    git('-c', 'user.name=Release test', '-c', 'user.email=release-test@example.invalid', 'commit', '--quiet', '-m', 'Fixture');
    const commit = git('rev-parse', 'HEAD').trim();
    const node = (name, args = []) => spawnSync(process.execPath, [path.join(root, 'scripts', name), ...args], { cwd: root, encoding: 'utf8', env: { ...process.env, GH_TOKEN: '', GITHUB_TOKEN: '' } });
    run({ parent, root, commit, node });
  } finally {
    const resolved = fs.realpathSync(parent), temp = fs.realpathSync(os.tmpdir());
    assert.ok(path.basename(resolved).startsWith('mewu-staging-') && path.dirname(resolved) === temp);
    fs.rmSync(resolved, { recursive: true });
  }
}
for (const version of [baselineVersion, '1.0.0', '1.0.1-preview.1']) test(`${version}: staging excludes private files and assembles three exact native receipts`, () => fixture(({ parent, root, commit, node }) => {
  const plan = releasePlan(version), request = { inputs: {}, receipts: {}, output: 'out/combined' };
  for (const platform of platforms) {
    const bundle = path.join(parent, platform); fs.mkdirSync(bundle);
    const installer = path.join(bundle, 'installer.bin'), updater = platform === 'darwin-aarch64' ? path.join(bundle, 'update.bin') : installer;
    fs.writeFileSync(installer, `fixture installer ${platform}`); fs.writeFileSync(updater, `fixture updater ${platform}`);
    const cliBinary = path.join(bundle, 'mewu.exe'); fs.writeFileSync(cliBinary, 'fixture console binary');
    fs.writeFileSync(path.join(bundle, 'private-key.pem'), 'adjacent secret fixture');
    fs.writeFileSync(path.join(bundle, 'agent.md'), 'adjacent notes fixture');
    const signature = Buffer.from(`untrusted comment: fixture\nfixture\ntrusted comment: timestamp:1\tversion:${version}\nfixture\n`).toString('base64');
    fs.writeFileSync(`${updater}.sig`, signature);
    const input = path.join(parent, `${platform}.json`), output = `out/${platform}`;
    fs.writeFileSync(input, JSON.stringify({ installer, updater, binary: installer, cliBinary }));
    // The Windows ZIP can also be constructed directly on non-Windows runners;
    // this fixture validates its bytes/receipt rather than a ZIP encoder.
    if (platform === 'windows-x86_64' && process.platform !== 'win32') {
      const directory = path.join(root, output); fs.mkdirSync(path.join(directory, '.metadata'), { recursive: true });
      for (const name of plan[platform].files) fs.writeFileSync(path.join(directory, name), 'windows fixture');
      fs.writeFileSync(path.join(directory, '.metadata/build.json'), JSON.stringify({ schemaVersion: 1, version, sourceCommit: commit, platform, assets: plan[platform].files.map(name => fingerprint(path.join(directory, name))), updater: { target: platform, name: plan[platform].updater, url: updateUrl(version, platform), signature } }));
    } else {
      const result = node('prepare-release.mjs', [output, platform, input]);
      assert.equal(result.status, 0, result.stderr);
    }
    const directory = path.join(root, output), names = [...new Set([...plan[platform].files, plan[platform].updater])];
    const artifacts = names.map(name => fingerprint(path.join(directory, name)));
    request.inputs[platform] = directory;
    const receipt = path.join(parent, `${platform}-receipt.json`); request.receipts[platform] = receipt;
    fs.writeFileSync(receipt, JSON.stringify({ schemaVersion: 1, platform, version, sourceCommit: commit, result: 'passed', nativeGuiVerified: true, installerVerified: true, nativeTests: { passed: 1, failed: 0 }, frontendTests: { passed: 1, failed: 0 }, checks: Object.fromEntries(requiredChecks.map(key => [key, 'passed'])), artifacts }));
    assert.deepEqual(fs.readdirSync(directory).sort(), [...names, '.metadata'].sort());
  }
  const file = path.join(parent, 'assemble.json'); fs.writeFileSync(file, JSON.stringify(request));
  const assembled = node('assemble-release.mjs', [file]); assert.equal(assembled.status, 0, assembled.stderr);
  const directory = path.join(root, request.output), meta = path.join(directory, '.metadata');
  const manifest = JSON.parse(fs.readFileSync(path.join(meta, 'update.json'))), acceptance = JSON.parse(fs.readFileSync(path.join(meta, 'acceptance.json')));
  const release = verifyRelease(directory, manifest, acceptance, commit);
  assert.equal(release.assets.length, githubReleaseUpdates(version) ? 5 : selfContainedRelease(version) ? 6 : 4);
  assert.equal(release.channelAssets.length, selfContainedRelease(version) ? 0 : 1);
  // Only negative publishing: removing Linux acceptance must fail locally,
  // before looking up GitHub or creating a draft/tag.
  delete acceptance.platforms['linux-x86_64']; fs.writeFileSync(path.join(meta, 'acceptance.json'), JSON.stringify(acceptance));
  const rejected = node('publish-release.mjs', [directory]); assert.notEqual(rejected.status, 0);
  assert.match(rejected.stderr, /Missing three-platform acceptance/);
}, version));

test('stable staging with a preview endpoint fails before reading installers or invoking GitHub', () => fixture(({ root, node }) => {
  const configFile = path.join(root, 'apps/desktop/src-tauri/tauri.conf.json');
  const config = JSON.parse(fs.readFileSync(configFile, 'utf8'));
  config.plugins.updater.endpoints = [publicationPolicy(baselineVersion).clientEndpoint];
  fs.writeFileSync(configFile, JSON.stringify(config));
  const rejected = node('prepare-release.mjs', ['out/refused', 'linux-x86_64']);
  assert.notEqual(rejected.status, 0); assert.match(rejected.stderr, /fixed signed release channel/);
  assert.equal(fs.existsSync(path.join(root, 'out')), false);
}, '1.0.0'));
