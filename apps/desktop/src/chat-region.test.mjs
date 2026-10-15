// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';

const load = async path => new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(path, import.meta.url), 'utf8'), { mode: 'transform' }));
const board = await load('./blackboard.ts'); await board.link(() => { throw Error('Unexpected import'); }); await board.evaluate();
const module = await load('./chat-region.ts'); await module.link(name => {
  assert.equal(name, './blackboard'); return board;
}); await module.evaluate();
const { chatRegion, chatRegionIdentity } = module.namespace;
const region = id => ({ id, x: 10, y: 20, width: 300, height: 200 });
const scene = { id: 'current', closed: false, frozen: false, background: { id: 'screen', kind: 'image', name: '截图.png', path: '/owned/screen.png' }, regions: [region('first'), region('second')] };
const ref = id => ({ kind: 'region', id });

test('only the explicitly referenced selected region accompanies a chat message', () => {
  assert.equal(chatRegion(scene, [ref('first')], 'first'), 'first');
  assert.equal(chatRegion(scene, [ref('first')], 'second'), undefined);
  assert.equal(chatRegion(scene, [{ kind: 'item', id: 'first' }], 'first'), undefined);
  assert.equal(chatRegion(scene, [], 'first'), undefined);
  assert.equal(chatRegion(scene, [ref('missing')], 'missing'), undefined);
});

test('multiple references never implicitly authorize the first region', () => {
  assert.equal(chatRegion(scene, [ref('first'), ref('second')]), undefined);
  assert.equal(chatRegion(scene, [ref('first'), ref('second')], 'second'), 'second');
  assert.equal(chatRegion(scene, [ref('second')]), 'second');
});

test('imported/long images, blackboards and inactive scenes are not live screen authority', () => {
  assert.equal(chatRegion({ ...scene, regions: [{ ...region('first'), imageOverride: { id: 'imported' } }] }, [ref('first')]), undefined);
  assert.equal(chatRegion({ ...scene, closed: true }, [ref('first')]), undefined);
  assert.equal(chatRegion({ ...scene, frozen: true }, [ref('first')]), undefined);
  assert.equal(chatRegion({ ...scene, background: undefined }, [ref('first')]), undefined);
  assert.equal(chatRegion({ ...scene, background: { id: 'board', kind: 'image', name: '黑板.png', path: '/owned/board.board.png' } }, [ref('first')]), undefined);
});

test('queued sends detect moved regions and replaced source pixels, without treating an ordinary snapshot clone as a change', () => {
  const identity = chatRegionIdentity(scene, 'first');
  assert.equal(chatRegionIdentity(structuredClone(scene), 'first'), identity);
  assert.notEqual(chatRegionIdentity({ ...scene, regions: [{ ...region('first'), x: 15 }] }, 'first'), identity);
  assert.notEqual(chatRegionIdentity({ ...scene, background: { ...scene.background, id: 'new-screen' } }, 'first'), identity);
  assert.equal(chatRegionIdentity(scene, 'missing'), undefined);
});
