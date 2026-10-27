// SPDX-License-Identifier: MPL-2.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { annotationEvidence, assertNativeRun } from './agent-eval-results.mjs';

test('grade stored rich objects and video primitives, not assistant claims', () => {
  const state = { items: [
    { imageAnnotations: { drawings: [{ kind: 'rect' }, { kind: 'rich', rich: { kind: 'formula' } }, { kind: 'rich', rich: { kind: 'table' } }] } },
    { videoAnnotations: { objects: [{ primitive: { kind: 'text' } }] } },
  ], messages: [{ role: 'assistant', text: '已画箭头' }] };
  assert.deepEqual(annotationEvidence(state), { objectCount: 4, kinds: ['rect', 'formula', 'table', 'text'] });
  assert.deepEqual(annotationEvidence({ items: [], messages: state.messages }), { objectCount: 0, kinds: [] });
});

test('native suite rejects an external engine or wrong connection', () => {
  assertNativeRun({ connectionId: 'model' }, 'model');
  assert.throws(() => assertNativeRun({ connectionId: 'model', externalRuntime: { pluginId: 'any-plugin' } }, 'model'), /external Agent/);
  assert.throws(() => assertNativeRun({ connectionId: 'other' }, 'model'), /different model/);
});
