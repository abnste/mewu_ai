// SPDX-License-Identifier: MPL-2.0
// Mandatory separate native suite: never silently skip when the validator is absent.
import assert from 'node:assert/strict';
import test from 'node:test';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createValidator, extensionFingerprint } from './validator.mjs';

if (!process.env.MEWU_PLUGIN_VALIDATOR) throw new Error('需要显式 MEWU_PLUGIN_VALIDATOR；本套测试不允许用 mock 或 skip 代替真实 Rust parser');
const validate = await createValidator(process.env.MEWU_PLUGIN_VALIDATOR);
const manifest = { schemaVersion: 2, id: 'thirdparty.example', version: '1.0.0', name: '真实原生契约测试', description: '格式校验不会运行插件',
  license: 'MPL-2.0', hostApi: 2, runtime: 'javascript', entry: 'index.js', permissions: ['storage.read'], tags: ['tools'], dependencies: {} };
const entry = { id: manifest.id, name: manifest.name, description: manifest.description, tags: manifest.tags, repository: 'example/plugin', version: '1.0.0',
  releaseTag: 'v1.0.0', assetName: 'plugin-1.0.0.zip', sha256: 'a'.repeat(64), size: 1024, hostApi: 2, platforms: ['any'] };
const encoded = value => Buffer.from(JSON.stringify(value));

test('the actual validator has no declaration-package or declaration-catalog compatibility modes', async () => {
  const execute = promisify(execFile);
  for (const mode of ['manifest', 'bundled-manifest', 'entry', 'catalog']) {
    await assert.rejects(execute(process.env.MEWU_PLUGIN_VALIDATOR, [mode], { windowsHide: true, timeout: 15_000, maxBuffer: 64 * 1024 }), error => {
      assert.equal(error.code, 1);
      const response = JSON.parse(error.stdout);
      assert.equal(response.ok, false); assert.equal(response.error, '不支持的校验模式');
      assert.equal('contractSha256' in response, false);
      return true;
    });
  }
});

test('actual host accepts the Mewu API manifest and preserves dynamic-registration contract', async () => {
  const value = await validate('extension-manifest', encoded(manifest));
  assert.equal(value.id, manifest.id); assert.equal(value.entry, 'index.js'); assert.deepEqual(value.permissions, manifest.permissions);
  assert.match(await extensionFingerprint(), /^[a-f0-9]{64}$/);
});

test('actual strict manifest parser rejects unknown fields, duplicate object keys and invalid UTF8/BOM', async () => {
  await assert.rejects(validate('extension-manifest', encoded({ ...manifest, contributions: [] })));
  await assert.rejects(validate('extension-manifest', Buffer.from(`{"id":"other",${JSON.stringify(manifest).slice(1)}`)));
  await assert.rejects(validate('extension-manifest', Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), encoded(manifest)])));
  await assert.rejects(validate('extension-manifest', Buffer.from([0xff, 0xfe])));
  for (const override of [{ schemaVersion: 1 }, { hostApi: 1 }, { runtime: 'node' }, { entry: '../index.js' }, { version: 'latest' }, { permissions: ['storage.read', 'storage.read'] }]) {
    await assert.rejects(validate('extension-manifest', encoded({ ...manifest, ...override })));
  }
});

test('actual native entry validates a fixed GitHub ZIP source and rejects substitutions', async () => {
  const value = await validate('extension-entry', encoded(entry));
  assert.equal(value.repository, entry.repository); assert.equal(value.sha256, entry.sha256);
  for (const override of [{ source: 'https://attacker.test/a.zip' }, { repository: 'https://github.com/example/plugin' }, { repository: 'example/../plugin' },
    { assetName: '../package.zip' }, { assetName: 'package.exe' }, { sha256: 'not-a-digest' }, { size: 0 }, { hostApi: 1 }, { platforms: ['unsupported'] }]) {
    await assert.rejects(validate('extension-entry', encoded({ ...entry, ...override })));
  }
  await assert.rejects(validate('extension-entry', Buffer.from(`{"repository":"attacker/repo",${JSON.stringify(entry).slice(1)}`)));
});

test('actual native catalog rejects duplicate identity, unknown root metadata and old schema', async () => {
  assert.equal((await validate('extension-catalog', encoded({ schemaVersion: 2, plugins: [entry] }))).plugins.length, 1);
  assert.equal((await validate('extension-catalog', encoded({ schemaVersion: 2, plugins: [] }))).plugins.length, 0);
  for (const value of [{ schemaVersion: 2, plugins: [entry, entry] }, { schemaVersion: 1, plugins: [entry] }, { schemaVersion: 2, plugins: [], installed: true }]) {
    await assert.rejects(validate('extension-catalog', encoded(value)));
  }
});

test('actual host accepts bounded vector icons in manifests and market entries without changing old packages', async () => {
  const icon = { paths: ['M4 4h16v16H4z', 'M8 8l8 8'], color: '#8b5cf6', filled: false };
  assert.deepEqual((await validate('extension-manifest', encoded({ ...manifest, icon }))).icon, icon);
  assert.deepEqual((await validate('extension-entry', encoded({ ...entry, icon }))).icon, icon);
  assert.equal('icon' in await validate('extension-manifest', encoded(manifest)), false);
  assert.equal('icon' in await validate('extension-entry', encoded(entry)), false);
  assert.equal((await validate('extension-entry', encoded({ ...entry, icon: { paths: ['M0 0L1 1'] } }))).icon.filled, false);
  const invalid = [
    { paths: [] }, { paths: Array(17).fill('M0 0') }, { paths: [' '] }, { paths: ['M'.repeat(4097)] },
    { paths: Array(3).fill('M'.repeat(3000)) }, { paths: ['<script>alert(1)</script>'] },
    { paths: ['M0 0'], color: 'url(https://example.com/icon)' }, { paths: ['M0 0'], color: '#fff' },
    { paths: ['M0 0'], filled: 'yes' }, { paths: ['M0 0'], onload: 'alert(1)' },
    { url: 'https://example.com/icon.svg' }, '<svg></svg>',
  ];
  for (const value of invalid) {
    await assert.rejects(validate('extension-manifest', encoded({ ...manifest, icon: value })));
    await assert.rejects(validate('extension-entry', encoded({ ...entry, icon: value })));
  }
});
