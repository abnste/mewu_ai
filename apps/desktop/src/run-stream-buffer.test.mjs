// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
import test from 'node:test';

const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./run-stream-buffer.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
await module.link(() => { throw Error('Unexpected runtime dependency'); }); await module.evaluate();
const { RunStreamBuffer } = module.namespace;
const scene = (id = 'scene', runId = 'run') => ({ id, closed: false, run: { id: runId, status: 'running' }, messages: [] });
function fixture(scenes = [scene()]) {
  const current = new Map(scenes.map(value => [value.id, value])), updates = [];
  return { current, updates, buffer: new RunStreamBuffer(id => current.get(id), value => updates.push(value)) };
}
const event = (text, extra = {}) => ({ sceneId: 'scene', runId: 'run', text, ...extra });

test('1000 reply fragments coalesce into one ordered update without mixing public reasoning', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const f = fixture();
  for (let index = 0; index < 1000; index++) f.buffer.push(event(`${index}🙂`, { reasoning: `思考${index}` }));
  assert.equal(f.updates.length, 0); t.mock.timers.tick(39); assert.equal(f.updates.length, 0);
  t.mock.timers.tick(1); assert.equal(f.updates.length, 1);
  assert.equal(f.updates[0].scene.text, Array.from({ length: 1000 }, (_, index) => `${index}🙂`).join(''));
  assert.equal(f.updates[0].scene.reasoning, Array.from({ length: 1000 }, (_, index) => `思考${index}`).join(''));
  f.buffer.dispose();
});

test('final snapshot releases duplicate caches and delayed deltas cannot resurrect the reply', () => {
  const f = fixture(); f.buffer.push(event('最后一段', { reasoning: '公开思考' }));
  const completed = { ...scene(), run: { id: 'run', status: 'completed' }, messages: [{ id: 'answer', role: 'assistant', runId: 'run', text: '完整答案', reasoning: '公开思考' }] };
  f.buffer.reconcile([completed]); f.current.set('scene', completed);
  assert.equal(f.updates[0].scene.text, '最后一段'); assert.deepEqual(f.updates.at(-1), {});
  f.buffer.push(event('迟到')); f.buffer.flush(); assert.deepEqual(f.updates.at(-1), {});
  assert.equal(completed.messages[0].text, '完整答案'); f.buffer.dispose();
});

test('cancellation keeps the last partial reply while rejecting later network fragments', () => {
  for (const status of ['failed', 'canceled']) {
    const f = fixture(); f.buffer.push(event('保留半段', { reasoning: '思考' }));
    const ended = { ...scene(), run: { id: 'run', status } };
    f.buffer.reconcile([ended]); f.current.set('scene', ended);
    f.buffer.push(event('不得出现')); f.buffer.flush();
    assert.equal(f.updates.at(-1).scene.text, '保留半段'); assert.equal(f.updates.at(-1).scene.reasoning, '思考');
    f.buffer.dispose();
  }
});

test('minimized and background runs remain independent without losing queued fragments', () => {
  const f = fixture([scene(), { ...scene('other', 'second'), frozen: true, minimized: true }]);
  f.buffer.push(event('A')); f.buffer.push(event('B', { sceneId: 'other', runId: 'second' }));
  f.buffer.reconcile([...f.current.values()]);
  assert.equal(f.updates.length, 1); assert.equal(f.updates[0].scene.text, 'A'); assert.equal(f.updates[0].other.text, 'B'); f.buffer.dispose();
});

test('close, new run, missing scene and explicit clearing revoke both pending and cached text', () => {
  for (const next of [{ ...scene(), closed: true }, scene('scene', 'replacement'), undefined]) {
    const f = fixture(); f.buffer.push(event('old')); f.buffer.flush(); f.buffer.push(event('pending'));
    f.buffer.reconcile(next ? [next] : []); if (next) f.current.set('scene', next); else f.current.delete('scene');
    assert.deepEqual(f.updates.at(-1), {}); f.buffer.push(event('late')); f.buffer.flush(); assert.deepEqual(f.updates.at(-1), {}); f.buffer.dispose();
  }
  const f = fixture(); f.buffer.push(event('cached')); f.buffer.flush(); f.buffer.push(event('pending')); f.buffer.clear('scene'); f.buffer.flush();
  assert.deepEqual(f.updates.at(-1), {}); f.buffer.dispose();
});

test('replacement between enqueue and flush rejects old run text; unmount cancels timers', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const f = fixture(); f.buffer.push(event('old')); f.current.set('scene', scene('scene', 'new'));
  f.buffer.push(event('new', { runId: 'new' })); t.mock.timers.tick(40);
  assert.deepEqual(f.updates.at(-1).scene, { runId: 'new', text: 'new', reasoning: '' });
  f.buffer.push(event('disposed', { runId: 'new' })); f.buffer.dispose(); t.mock.timers.tick(100);
  assert.equal(f.updates.length, 1); f.buffer.push(event('ignored')); f.buffer.flush(); assert.equal(f.updates.length, 1);
});
