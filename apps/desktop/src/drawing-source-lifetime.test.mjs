// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
import { parse } from '@babel/parser';

const editor = await readFile(new URL('./components/SharedDrawingEditor.tsx', import.meta.url), 'utf8');
const ast = parse(editor, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const body = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body;
const source = body.flatMap(node => node.declarations ?? []).find(node => node.id.name === 'strokeSource').init;
const sourceOf = Function('props', 'cacheKey', `return (${editor.slice(source.start, source.end)})();`);
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./components/drawing-history.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
await module.link(() => { throw Error('Unexpected import'); }); await module.evaluate();
const { DrawingHistoryLane } = module.namespace;
const tick = () => new Promise(resolve => setImmediate(resolve));

function fixture() {
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  const props = { box: { x: 10, y: 20, width: 900, height: 540 },
    port: { sourceIdentity: () => 'image-asset', sourceSize: () => ({ width: 900, height: 540 }), frame: () => ({ x: 0, y: 0, width: 900, height: 540 }) } };
  const readSource = () => sourceOf(props, () => 'opaque-image-editor-key');
  const original = readSource();
  let state = { revision: 0, drawings: [], undo: [], redo: [] };
  const drafts = new Map(), failures = [];
  const lane = new DrawingHistoryLane({
    read: () => ({ ...state, source: readSource() }), active: () => readSource() === original,
    changed() {}, completed() {}, failed: error => failures.push(error),
    save: async () => { throw Error('Unexpected history operation'); },
    add: {
      remember: drawing => drafts.set(drawing.id, drawing), forget: drawing => drafts.delete(drawing.id),
      save: async drawing => {
        await gate;
        state = { revision: state.revision + 1, drawings: [...state.drawings, drawing],
          undo: [...state.undo, { index: state.drawings.length, before: null, after: drawing }], redo: [] };
      },
    },
  });
  return { props, readSource, lane, release, drafts, failures, state: () => state };
}
const stroke = id => ({ id, kind: 'pen', color: '#EE4848', strokeWidth: 4, points: [{ x: 10, y: 20 }, { x: 40, y: 50 }] });

test('moving/rescaling a drawn image while its save returns does not strand drafts or block subsequent output', async () => {
  const f = fixture();
  assert.equal(f.lane.enqueueAdd(stroke('first'), f.lane.revisionIdentity()), true);
  await tick();
  f.props.box = { x: -100, y: 250, width: 450, height: 270 };
  f.release(); await f.lane.drain();
  assert.equal(f.drafts.size, 0);
  assert.equal(f.failures.length, 0);
  for (let index = 0; index < 8; index++) {
    f.props.box = { ...f.props.box, x: index * 30, width: 450 + index * 15 };
    assert.equal(f.lane.enqueueAdd(stroke(`repeat-${index}`), f.lane.revisionIdentity()), true);
    await f.lane.drain(); // The same flush required by copy, pin and minimize.
    assert.equal(f.drafts.size, 0);
  }
  assert.equal(f.state().drawings.length, 9);
});

test('replacing actual source pixels during a save still rejects the handoff and retains unconfirmed ink', async () => {
  const f = fixture();
  assert.equal(f.lane.enqueueAdd(stroke('first'), f.lane.revisionIdentity()), true);
  await tick();
  f.props.port.sourceIdentity = () => 'replacement-asset';
  f.release(); await assert.rejects(f.lane.drain());
  assert.equal(f.drafts.size, 1);
  assert.equal(f.failures.length, 1);
});

test('content geometry changes still invalidate the stroke document while viewport geometry does not', () => {
  const f = fixture(), original = f.readSource();
  f.props.box = { x: 0, y: 0, width: 0, height: 0 };
  assert.equal(f.readSource(), original);
  f.props.port.frame = () => ({ x: 10, y: 0, width: 890, height: 540 });
  assert.notEqual(f.readSource(), original);
});
