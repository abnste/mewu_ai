// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
const source = stripTypeScriptTypes(await readFile(new URL('./video-placement.ts', import.meta.url), 'utf8'), { mode: 'transform' });
const { observeVideoComposer, observeVideoPlacementTransitions, placeVideoToolbars, placeVideoControls } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const overlap = (a, b, gap = 0) => a.left < b.left + b.width && a.left + a.width > b.left && a.top < b.top + b.height + gap && a.top + a.height + gap > b.top;
function check(box, viewport, actionSize, trimHeight, composer, controls) {
  const placed = placeVideoToolbars(box, viewport, actionSize, trimHeight, composer, controls);
  const actions = { ...placed.actions, height: actionSize.height }, timeline = { ...placed.timeline, height: trimHeight };
  assert.ok(actions.top + actions.height + 6 <= timeline.top, 'action and timeline bubbles keep their order and gap');
  for (const value of [actions, timeline]) {
    assert.ok(value.left >= 8 && value.top >= 8);
    assert.ok(value.left + value.width <= viewport.width - 8);
    assert.ok(value.top + value.height <= viewport.height - 8);
    for (const obstacle of [composer, controls].filter(Boolean)) assert.equal(overlap(value, obstacle, 5), false, 'both bubbles avoid real composer/blackboard controls');
  }
  return placed;
}
test('normal video keeps object actions above and timeline below as separate bubbles', () => {
  const box = { left: 200, top: 200, width: 640, height: 360 }, screen = { width: 1280, height: 900 };
  const placed = check(box, screen, { width: 318, height: 58 }, 64, { left: 320, top: 800, width: 640, height: 100 });
  assert.equal(placed.actions.top, 136); assert.equal(placed.timeline.top, 566);
});
test('a bottom-edge video cannot make both independent fallbacks occupy the same slot', () => {
  const box = { left: 180, top: 415, width: 450, height: 150 }, screen = { width: 900, height: 620 };
  const composer = { left: 200, top: 510, width: 500, height: 102 }, size = { width: 370, height: 58 };
  const oldActions = placeVideoControls(box, screen, size.height, composer, undefined, { side: 'above', width: size.width });
  const oldTrim = placeVideoControls(box, screen, 108, composer);
  assert.equal(overlap({ ...oldActions, height: 58 }, { ...oldTrim, height: 108 }), true, 'independent placement reproduces the reported collision');
  check(box, screen, size, 108, composer);
});
test('narrow two-row timeline and top-edge actions jointly avoid a moving composer and blackboard toolbar', () => {
  for (const y of [4, 75, 210, 410]) for (const height of [64, 120, 174]) {
    check({ left: 4, top: y, width: 340, height: 150 }, { width: 380, height: 680 }, { width: 360, height: 58 }, height,
      { left: 8, top: 570, width: 364, height: 102 }, { left: 8, top: 494, width: 364, height: 64 });
  }
});
test('a composer beside the video does not push toolbars away without a horizontal intersection', () => {
  const box = { left: 5, top: 180, width: 300, height: 180 };
  const without = placeVideoToolbars(box, { width: 1200, height: 800 }, { width: 280, height: 58 }, 108);
  const beside = check(box, { width: 1200, height: 800 }, { width: 280, height: 58 }, 108, { left: 600, top: 120, width: 450, height: 500 });
  assert.deepEqual(beside, without);
});

test('composer translation completion and interruption remeasure placement without size changes, then detach', () => {
  const composer = new EventTarget(); let calls = 0;
  const stop = observeVideoPlacementTransitions(composer, () => calls++);
  const send = (name, propertyName) => composer.dispatchEvent(Object.assign(new Event(name), { propertyName }));
  send('transitionend', 'translate'); assert.equal(calls, 1);
  send('transitioncancel', 'translate'); assert.equal(calls, 2);
  send('transitionend', 'opacity'); assert.equal(calls, 2);
  stop(); send('transitionend', 'translate'); assert.equal(calls, 2);
});

test('a late Composer mount, removal and replacement bind only live node observers and transitions', () => {
  const previous = [globalThis.MutationObserver, globalThis.ResizeObserver], mutations = [], sizes = [];
  class Observer {
    constructor(callback, list) { this.callback = callback; this.list = list; list.push(this); }
    observe(target, options) { this.target = target; this.options = options; this.active = true; }
    disconnect() { this.active = false; }
    notify() { if (this.active) this.callback([]); }
  }
  globalThis.MutationObserver = class extends Observer { constructor(callback) { super(callback, mutations); } };
  globalThis.ResizeObserver = class extends Observer { constructor(callback) { super(callback, sizes); } };
  let composer = null, calls = 0, stop;
  const root = { querySelector(selector) { assert.equal(selector, ':scope > .composer'); return composer; } };
  const transition = node => node.dispatchEvent(Object.assign(new Event('transitionend'), { propertyName: 'translate' }));
  try {
    stop = observeVideoComposer(root, () => calls++);
    assert.deepEqual(mutations[0].options, { childList: true }, 'never observe streaming descendants or the full document');
    assert.equal(calls, 0); assert.equal(sizes.length, 0);
    const first = new EventTarget(); composer = first; mutations[0].notify(); assert.equal(calls, 1);
    assert.equal(mutations[1].target, first); assert.equal(sizes[0].target, first);
    transition(first); mutations[1].notify(); sizes[0].notify(); assert.equal(calls, 4);
    mutations[0].notify(); assert.equal(calls, 4, 'unrelated app siblings do not trigger layout');
    composer = null; mutations[0].notify(); assert.equal(calls, 5);
    assert.equal(mutations[1].active, false); assert.equal(sizes[0].active, false);
    transition(first); mutations[1].callback([]); sizes[0].callback([]); assert.equal(calls, 5, 'retired or queued callbacks cannot reposition the live view');
    const second = new EventTarget(); composer = second; mutations[0].notify(); assert.equal(calls, 6);
    assert.equal(mutations[2].target, second); assert.equal(sizes[1].target, second);
    transition(first); assert.equal(calls, 6); transition(second); assert.equal(calls, 7);
    stop(); transition(second); mutations[0].callback([]); mutations[2].callback([]); sizes[1].callback([]);
    assert.equal(calls, 7); assert.ok(mutations.every(value => !value.active)); assert.ok(sizes.every(value => !value.active));
  } finally { stop?.(); [globalThis.MutationObserver, globalThis.ResizeObserver] = previous; }
});
