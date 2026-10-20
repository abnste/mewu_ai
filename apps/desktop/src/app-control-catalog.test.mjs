// SPDX-License-Identifier: MPL-2.0
// Catalog examples pass through the actual production adapter parser here.
// Full draft-2020-12 validation is in app_control_catalog_tests.rs using the
// existing native jsonschema crate, not a second handwritten schema validator.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
const catalog = JSON.parse(await readFile(new URL('../control/catalog.json', import.meta.url), 'utf8'));
const source = await readFile(new URL('./app-control.ts', import.meta.url), 'utf8');
const api = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source, { mode: 'transform' })).toString('base64')}`);
const queryMethods = ['app.status', 'app.schema', 'app.capabilities', 'scene.list', 'scene.get', 'run.get', 'agent.list', 'model.list', 'plugin.list', 'recording.status', 'operation.get'];

test('actual JSON catalog contains exactly 26 implemented UI operations and 11 native queries', () => {
  assert.equal(catalog.protocolVersion, 1); assert.equal(catalog.pluginId, 'mewu.cli'); assert.equal(catalog.automaticRetries, false);
  assert.equal(catalog.commands.length, 37);
  assert.equal(new Set(catalog.commands.map(command => command.method)).size, catalog.commands.length);
  assert.deepEqual(catalog.commands.filter(command => command.execution === 'operation').map(command => command.method).sort(), [...api.appControlMethods].sort());
  assert.deepEqual(catalog.commands.filter(command => command.execution === 'query').map(command => command.method).sort(), queryMethods.sort());
  assert.ok(Buffer.byteLength(JSON.stringify(catalog)) < 256 * 1024, 'schema query fits the actual host result budget');
  for (const name of ['Region', 'Drawing', 'Refs', 'OcrTarget', 'Context']) assert.equal(typeof catalog.$defs[name], 'object');
});

test('every UI example is accepted by the production request parser after native context extraction', () => {
  for (const command of catalog.commands) {
    assert.ok(command.description.trim()); assert.equal(command.asyncOperation, command.execution === 'operation');
    assert.equal(command.inputSchema.type, 'object'); assert.equal(command.inputSchema.additionalProperties, false);
    assert.ok(command.resultSchema); assert.ok(command.examples.length);
    for (const example of command.examples) {
      const { id, method, params } = structuredClone(example.request);
      assert.equal(method, command.method);
      if (command.execution !== 'operation') { assert.equal('context' in command.inputSchema.properties, false); continue; }
      const context = params.context ?? {}; delete params.context;
      const decoded = api.parseAppControlRequest({ id, nonce: 'host-generated-example-nonce', method, params, ...context });
      assert.equal(decoded.method, method);
      assert.deepEqual(decoded.params, params);
    }
  }
});

test('selection examples use actual official plugin contribution identities and tell callers to read current revisions', async () => {
  const names = { 'ocr.run': 'ocr', 'scroll.start': 'scroll', 'translation.run': 'translation', 'pin.create': 'pin' };
  for (const [method, name] of Object.entries(names)) {
    const manifest = JSON.parse(await readFile(new URL(`../plugins/official-${name}.json`, import.meta.url), 'utf8'));
    const command = catalog.commands.find(command => command.method === method), example = command.examples[0];
    assert.equal(example.request.params.pluginId, manifest.id);
    assert.ok(manifest.contributions.some(contribution => contribution.id === example.request.params.contributionId));
    assert.ok(example.description.includes('plugin.list'));
  }
});
