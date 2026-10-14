// SPDX-License-Identifier: MPL-2.0
// Mandatory separate native suite: never silently skip when the validator is absent.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createValidator, extensionFingerprint } from './validator.mjs';

if (!process.env.MEWU_PLUGIN_VALIDATOR) throw new Error('需要显式 MEWU_PLUGIN_VALIDATOR；本套测试不允许用 mock 或 skip 代替真实 Rust parser');
const validate = await createValidator(process.env.MEWU_PLUGIN_VALIDATOR);
const manifest = { schemaVersion: 2, id: 'thirdparty.example', version: '1.0.0', name: '真实原生契约测试', description: '格式校验不会运行插件',
  license: 'MPL-2.0', hostApi: 2, runtime: 'javascript', entry: 'index.js', permissions: ['storage.read'], tags: ['tools'], dependencies: {} };
const entry = { id: manifest.id, name: manifest.name, description: manifest.description, tags: manifest.tags, repository: 'example/plugin', version: '1.0.0',
  releaseTag: 'v1.0.0', assetName: 'plugin-1.0.0.zip', sha256: 'a'.repeat(64), size: 1024, hostApi: 2, platforms: ['any'] };
const encoded = value => Buffer.from(JSON.stringify(value));

test('actual host accepts API 2 manifest and preserves dynamic-registration contract', async () => {
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
