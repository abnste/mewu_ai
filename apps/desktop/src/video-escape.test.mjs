// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';
import test from 'node:test';

const videoSource = await readFile(new URL('./components/VideoArtifact.tsx', import.meta.url), 'utf8');
const appSource = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
function find(ast, predicate) {
  if (!ast || typeof ast !== 'object') return undefined;
  if (predicate(ast)) return ast;
  for (const value of Object.values(ast)) {
    for (const child of Array.isArray(value) ? value : [value]) {
      const result = find(child, predicate);
      if (result) return result;
    }
  }
}
const video = find(parse(videoSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] }), node => node.type === 'JSXOpeningElement' && node.name.name === 'video');
const keydown = video.attributes.find(node => node.name?.type === 'JSXNamespacedName' && node.name.namespace.name === 'on' && node.name.name.name === 'keydown').value.expression;
const escape = find(parse(appSource, { sourceType: 'module', plugins: ['typescript', 'jsx'] }), node => node.type === 'VariableDeclarator' && node.id.name === 'escape').init;
function fixture(minimized = false) {
  const calls = [];
  const scope = { pausePlayback: () => calls.push('pause'), toggle: () => calls.push('toggle'), openDrawing: () => calls.push('drawing'), nativeSelectOwnsEscape: () => false, exitPreparing: () => false, settings: () => false, pendingDrawingDrafts: () => [], scroll: () => undefined, recording: () => undefined, busy: () => false, sessionsOpen: () => false, scene: () => ({ id: 'owned-video-scene', minimized }), freeze: () => calls.push('freeze'), closeScene: id => calls.push(`close:${id}`) };
  vm.createContext(scope);
  vm.runInContext(stripTypeScriptTypes(`videoKey = (${videoSource.slice(keydown.start, keydown.end)}); appEscape = (${appSource.slice(escape.start, escape.end)});`, { mode: 'transform' }), scope);
  function dispatch(key) {
    const event = { key, isComposing: false, ctrlKey: false, altKey: false, metaKey: false, stopped: false, preventDefault() {}, stopPropagation() { this.stopped = true; } };
    scope.videoKey(event);
    if (!event.stopped) scope.appEscape(event);
    return event;
  }
  return { calls, dispatch };
}
test('actual focused video Escape pauses and reaches ordinary session close', () => {
  const f = fixture();
  assert.equal(f.dispatch('Escape').stopped, false);
  assert.deepEqual(f.calls, ['pause', 'close:owned-video-scene']);
});
test('actual focused video Escape preserves minimized session freeze', () => {
  const f = fixture(true);
  f.dispatch('Escape');
  assert.deepEqual(f.calls, ['pause', 'freeze']);
});
test('actual focused video space and drawing shortcut remain owned by video', () => {
  const f = fixture();
  assert.equal(f.dispatch(' ').stopped, true);
  assert.equal(f.dispatch('d').stopped, true);
  assert.deepEqual(f.calls, ['toggle', 'drawing']);
});
