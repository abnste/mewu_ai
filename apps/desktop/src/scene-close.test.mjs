// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';
import { parse } from '@babel/parser';

const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
function product(name) {
  let found;
  function walk(node) {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'FunctionDeclaration' && node.id?.name === name) found = node;
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) value.forEach(walk); else walk(value);
    }
  }
  walk(ast); assert.ok(found, name);
  return stripTypeScriptTypes(source.slice(found.start, found.end), { mode: 'transform' });
}
const { AppControlError } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL('./app-control.ts', import.meta.url), 'utf8'), { mode: 'transform' })).toString('base64')}`);
function fixture(fail) {
  const scene = { id: 'scene', closed: false }, calls = [], errors = [];
  let busy = false, closing, operations = 0;
  const step = name => async () => { calls.push(name); if (fail === name) throw Error('private-path-or-key'); };
  const scope = {
    AppControlError, disposed: false, autosaveVersion: 0, debounce: undefined,
    clearTimeout, voice: { cancel: step('voice') }, scene: () => scene,
    objectFlush: { flush: step('objects') }, flushVideoDrawing: step('videoDrawing'),
    flushVideo: step('video'), flushDrawing: step('drawing'), regionGeometry: { flush: step('geometry') },
    draft: () => 'unsaved text', refs: () => [], commands: Promise.resolve(),
    snapshot: () => ({ activeSceneId: scene.id, scenes: [scene] }),
    pendingSceneDrafts: () => [{ type: 'set_draft' }], command: step('draft'),
    busy: () => busy, recording: () => null, scroll: () => null, exitPreparing: () => false,
    spaceOperations: { begin() { operations++; return () => { operations--; }; } },
    setBusy: value => { busy = value; }, setClosingScene: value => { closing = value; },
    bridge: { closeSceneWindow: async () => { calls.push('close'); scene.closed = true; return {}; } },
    accept() {}, clearSceneState() { calls.push('clear'); }, setError() {}, setSessionsOpen() {}, setSelectionActive() {},
    showError: value => errors.push(value),
  };
  vm.createContext(scope); vm.runInContext(product('flush') + '\n' + product('closeScene'), scope);
  return { scope, calls, errors, scene, unlock: () => { fail = undefined; }, state: () => ({ busy, closing, operations }) };
}

test('every authoring failure preserves the scene, releases close ownership and reports a safe stage', async () => {
  for (const [stage, label] of Object.entries({ voice: '语音输入', objects: '素材编辑', videoDrawing: '视频绘制', video: '视频编辑', drawing: '截图标注', geometry: '选区位置', draft: '会话草稿' })) {
    const f = fixture(stage);
    await assert.rejects(f.scope.closeScene('scene', true), error => {
      assert.equal(error.code, 'OPERATION_FAILED');
      assert.equal(error.message, `${label}未保存，当前会话已保留`);
      assert.equal(error.cause.message, 'private-path-or-key');
      assert.equal(JSON.stringify(error).includes('private-path-or-key'), false);
      return true;
    });
    assert.equal(f.scene.closed, false); assert.equal(f.calls.includes('clear'), false);
    assert.deepEqual(f.state(), { busy: false, closing: undefined, operations: 0 });
    f.unlock(); assert.equal(await f.scope.closeScene('scene', true), true);
    assert.equal(f.calls.filter(x => x === 'close').length, 1);
  }
});

test('a slow real save retains close ownership and duplicate Escape cannot repeat it', async () => {
  const f = fixture(); let release;
  f.scope.objectFlush.flush = () => new Promise(resolve => { release = resolve; });
  const closing = f.scope.closeScene('scene');
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(await f.scope.closeScene('scene'), false);
  assert.equal(f.state().busy, true); assert.equal(f.calls.includes('close'), false);
  release(); assert.equal(await closing, true);
  assert.deepEqual(f.state(), { busy: false, closing: undefined, operations: 0 });
});
