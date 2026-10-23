// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
const plain = async path => stripTypeScriptTypes(await readFile(new URL(path, import.meta.url), 'utf8'), {mode: 'transform'});
const dataUrl = code => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
// extension-triggers imports extension-state at runtime; point that relative
// specifier at the transformed module before importing from a data URL.
const stateUrl = dataUrl(await plain('./extension-state.ts'));
const triggers = await import(dataUrl((await plain('./extension-triggers.ts')).replace("'./extension-state'", JSON.stringify(stateUrl))));
const state = await import(stateUrl);

const altOnly = {ctrl: false, shift: false, alt: true};
const service = (id, metadata) => ({kind: 'trigger', id, title: id, metadata});
const record = (services, permissions = ['input.trigger'], pluginId = 'example.hold') => ({
  manifest: {id: pluginId, permissions}, revision: 3, state: 'enabled', services
});

test('descriptor normalization accepts modifier sets and rejects everything else', () => {
  assert.deepEqual(triggers.normalizeTriggerDescriptor(service('alt', {trigger: {code: 'Alt'}})),
    {id: 'alt', code: 'Alt', mask: 4});
  assert.deepEqual(triggers.normalizeTriggerDescriptor(service('combo', {trigger: {code: 'Control', with: ['Alt']}})),
    {id: 'combo', code: 'Control', with: ['Alt'], mask: 5});
  assert.deepEqual(triggers.normalizeTriggerDescriptor(service('empty', {trigger: {code: 'Shift', with: []}})),
    {id: 'empty', code: 'Shift', mask: 2});
  for (const metadata of [
    {trigger: {code: 'KeyA'}}, {trigger: {code: 'a'}}, {trigger: {code: 'Enter'}},
    {trigger: {}}, {trigger: {code: 'Alt', with: ['Alt']}}, {trigger: {code: 'Alt', with: ['Meta']}},
    {trigger: {code: 'Alt', with: ['Ctrl', 'Ctrl']}}, {trigger: {code: 'Control', with: 'Alt'}}, {},
  ]) assert.equal(triggers.normalizeTriggerDescriptor(service('bad', metadata)), undefined, JSON.stringify(metadata));
  assert.equal(triggers.normalizeTriggerDescriptor({kind: 'trigger', id: 'bare', title: 'bare'}), undefined);
  assert.equal(triggers.normalizeTriggerDescriptor(service('nonttrigger', {trigger: 'Alt'})), undefined);
});

test('matching requires exact modifier set equality, not a subset', () => {
  const alt = triggers.normalizeTriggerDescriptor(service('alt', {trigger: {code: 'Alt'}}));
  const combo = triggers.normalizeTriggerDescriptor(service('combo', {trigger: {code: 'Control', with: ['Alt']}}));
  assert.equal(triggers.triggerModifiersMatch(alt, altOnly), true);
  // Ctrl+Alt held must not fire a bare Alt trigger.
  assert.equal(triggers.triggerModifiersMatch(alt, {ctrl: true, shift: false, alt: true}), false);
  assert.equal(triggers.triggerModifiersMatch(alt, triggers.noModifiers), false);
  assert.equal(triggers.triggerModifiersMatch(combo, altOnly), false);
  assert.equal(triggers.triggerModifiersMatch(combo, {ctrl: true, shift: false, alt: true}), true);
  assert.equal(triggers.triggerModifiersMatch(combo, {ctrl: true, shift: true, alt: true}), false);
});

test('trigger payload carries phase, optional reason, pointer and modifiers only', () => {
  const pointer = {x: 12.5, y: -3, buttons: 2};
  assert.deepEqual(triggers.triggerInput('down', altOnly, pointer),
    {type: 'trigger', phase: 'down', pointer, modifiers: altOnly});
  assert.deepEqual(triggers.triggerInput('cancel', triggers.noModifiers, pointer, 'blur'),
    {type: 'trigger', phase: 'cancel', reason: 'blur', pointer, modifiers: triggers.noModifiers});
});

test('catalog selector requires the trigger kind and the input.trigger permission', () => {
  const records = [
    record([service('ok', {trigger: {code: 'Alt'}})]),
    {...record([service('no-permission', {trigger: {code: 'Alt'}})], []), manifest: {id: 'plain', permissions: []}},
    record([{kind: 'command', id: 'cmd', title: 'cmd'}]),
    {...record([service('disabled', {trigger: {code: 'Alt'}})]), state: 'disabled'},
  ];
  const chosen = state.triggerExtensionServices(records);
  assert.deepEqual(chosen.map(choice => choice.service.id), ['ok']);
});

function space(records, sceneId = () => 'scene-1') {
  const listeners = new Map(), removed = [], invocations = [];
  const window = {
    addEventListener(name, fn, options) { listeners.set(name, {fn, options}); },
    removeEventListener(name, fn) { removed.push(name); },
  };
  const previous = globalThis.window;
  globalThis.window = window;
  let attached = true;
  const detach = triggers.attachExtensionTriggers({sceneId, records: () => records, invoke: invocation => { invocations.push(invocation); return Promise.resolve(null); }});
  const send = (name, event) => {
    const listener = listeners.get(name);
    if (!listener || !attached) return {prevented: false};
    let prevented = false;
    listener.fn({preventDefault() { prevented = true; }, stopPropagation() {}, ...event});
    return {prevented};
  };
  return {
    invocations,
    keydown: event => send('keydown', {repeat: false, isComposing: false, ctrlKey: false, shiftKey: false, altKey: false, ...event}),
    keyup: event => send('keyup', {ctrlKey: false, shiftKey: false, altKey: false, ...event}),
    blur: () => send('blur', {}),
    pointermove: event => send('pointermove', {buttons: 0, ...event}),
    close() { attached = false; detach(); globalThis.window = previous; return removed; },
  };
}
const inputs = invocations => invocations.map(({pluginId, revision, serviceId, sceneId, input}) => ({pluginId, revision, serviceId, sceneId, input}));

test('hold lifecycle dispatches down and up with pointer state to matching triggers only', () => {
  const held = space([
    record([service('alt', {trigger: {code: 'Alt'}}), service('combo', {trigger: {code: 'Control', with: ['Alt']}})]),
    record([service('foreign-alt', {trigger: {code: 'Alt'}})], ['input.trigger'], 'other.plugin'),
    record([service('unpermitted', {trigger: {code: 'Alt'}})], [], 'unpermitted.plugin'),
  ]);
  try {
    held.keydown({key: 'Alt', altKey: true});
    assert.deepEqual(inputs(held.invocations), [
      {pluginId: 'example.hold', revision: 3, serviceId: 'alt', sceneId: 'scene-1',
        input: {type: 'trigger', phase: 'down', pointer: {x: 0, y: 0, buttons: 0}, modifiers: altOnly}},
      {pluginId: 'other.plugin', revision: 3, serviceId: 'foreign-alt', sceneId: 'scene-1',
        input: {type: 'trigger', phase: 'down', pointer: {x: 0, y: 0, buttons: 0}, modifiers: altOnly}},
    ]);
    held.pointermove({clientX: 320.5, clientY: 40, buttons: 1});
    held.keyup({key: 'Alt', ctrlKey: false});
    assert.equal(held.invocations.length, 4);
    assert.deepEqual(held.invocations[2].input,
      {type: 'trigger', phase: 'up', pointer: {x: 320.5, y: 40, buttons: 1}, modifiers: triggers.noModifiers});
    assert.deepEqual(held.invocations[3].input.phase, 'up');
    // The session ended; a later keyup cannot dispatch twice.
    held.keyup({key: 'Alt'});
    assert.equal(held.invocations.length, 4);
  } finally { held.close(); }
});

test('combo holds fire only on the completing modifier and end when any member is released', () => {
  const combo = space([record([service('combo', {trigger: {code: 'Control', with: ['Alt']}})])]);
  try {
    combo.keydown({key: 'Control', ctrlKey: true});
    assert.equal(combo.invocations.length, 0);
    // A non-modifier keydown never arms even when the set already matches.
    combo.keydown({key: 'x', ctrlKey: true, altKey: true});
    assert.equal(combo.invocations.length, 0);
    combo.keydown({key: 'Alt', ctrlKey: true, altKey: true});
    assert.equal(combo.invocations.length, 1);
    assert.equal(combo.invocations[0].input.phase, 'down');
    // Shift is not part of the held set; releasing it keeps the session.
    combo.keyup({key: 'Shift', ctrlKey: true, altKey: true, shiftKey: true});
    assert.equal(combo.invocations.length, 1);
    combo.keyup({key: 'Alt', ctrlKey: true});
    assert.deepEqual(combo.invocations[1].input,
      {type: 'trigger', phase: 'up', pointer: {x: 0, y: 0, buttons: 0}, modifiers: {ctrl: true, shift: false, alt: false}});
  } finally { combo.close(); }
});

test('repeat and composition keydowns are ignored while arming and holding', () => {
  const held = space([record([service('alt', {trigger: {code: 'Alt'}})])]);
  try {
    held.keydown({key: 'Alt', altKey: true, repeat: true});
    held.keydown({key: 'Alt', altKey: true, isComposing: true});
    assert.equal(held.invocations.length, 0);
    held.keydown({key: 'Alt', altKey: true});
    held.keydown({key: 'Alt', altKey: true, repeat: true});
    assert.equal(held.invocations.length, 1);
    held.keyup({key: 'Alt'});
  } finally { held.close(); }
});

test('escape during a hold is swallowed and dispatches cancel with the escape reason', () => {
  const held = space([record([service('alt', {trigger: {code: 'Alt'}})])], () => 'scene-1');
  try {
    held.keydown({key: 'Alt', altKey: true});
    const escape = held.keydown({key: 'Escape', altKey: true});
    assert.equal(escape.prevented, true);
    assert.equal(held.invocations.length, 2);
    assert.deepEqual(held.invocations[1].input,
      {type: 'trigger', phase: 'cancel', reason: 'escape', pointer: {x: 0, y: 0, buttons: 0}, modifiers: altOnly});
    held.keyup({key: 'Alt'});
    assert.equal(held.invocations.length, 2);
  } finally { held.close(); }
  // Without an armed hold, Escape stays with the rest of the application.
  const idle = space([record([service('alt', {trigger: {code: 'Alt'}})])]);
  try {
    assert.equal(idle.keydown({key: 'Escape'}).prevented, false);
    assert.equal(idle.invocations.length, 0);
  } finally { idle.close(); }
});

test('window blur cancels the hold and cannot dispatch afterwards', () => {
  const held = space([record([service('alt', {trigger: {code: 'Alt'}})])]);
  try {
    held.keydown({key: 'Alt', altKey: true});
    held.blur();
    assert.deepEqual(held.invocations[1].input,
      {type: 'trigger', phase: 'cancel', reason: 'blur', pointer: {x: 0, y: 0, buttons: 0}, modifiers: triggers.noModifiers});
    held.keyup({key: 'Alt'});
    assert.equal(held.invocations.length, 2);
  } finally { held.close(); }
});

test('holds without a live scene arm nothing and detach removes every listener', () => {
  const sceneless = space([record([service('alt', {trigger: {code: 'Alt'}})])], () => undefined);
  try {
    sceneless.keydown({key: 'Alt', altKey: true});
    assert.equal(sceneless.invocations.length, 0);
  } finally {
    const removed = sceneless.close();
    assert.deepEqual([...removed].sort(), ['blur', 'keydown', 'keyup', 'pointermove']);
  }
});
