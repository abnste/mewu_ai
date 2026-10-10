// Actual Composer handlers with synthetic state only; no native IPC or models.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import test from 'node:test';
import { parse } from '@babel/parser';

const source = await readFile(new URL('components/Composer.tsx', import.meta.url), 'utf8');
const tree = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const component = tree.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration;
const send = component.body.body.flatMap(node => node.type === 'VariableDeclaration' ? node.declarations : []).find(node => node.id.name === 'send').init;
const elements = [];
function visit(node) {
  if (!node || typeof node !== 'object') return;
  if (node.type === 'JSXElement') elements.push(node);
  for (const value of Object.values(node)) {
    if (Array.isArray(value)) value.forEach(visit);
    else if (value && typeof value === 'object') visit(value);
  }
}
visit(component);
const attribute = (element, name) => element.openingElement.attributes.find(node => node.type === 'JSXAttribute' && node.name.name === name)?.value;
const textarea = elements.find(node => node.openingElement.name.name === 'textarea');
const button = elements.find(node => attribute(node, 'class')?.value === 'composer-round composer-send');
const expression = node => source.slice(node.start, node.end);

function fixture(overrides = {}) {
  const sent = [], canceled = [], revealed = [], atEnd = [];
  const props = { scene: {}, draft: '请解释这张图', refs: [], sending: false, closing: false,
    onSend: (...args) => sent.push(args), onCancel: () => canceled.push(true), ...overrides };
  const actual = runInNewContext(`
    const running = () => props.scene.run?.status === 'running';
    let focusAnchor;
    const send = ${expression(send)};
    ({ send, keydown: ${expression(attribute(textarea, 'onKeyDown').expression)},
      click: ${expression(attribute(button, 'onClick').expression)},
      disabled: () => ${expression(attribute(button, 'disabled').expression)}, focusAnchor: () => focusAnchor });
  `, { props, lastPointer: { x: 42, y: 18 }, setAtEnd: value => atEnd.push(value), reveal: () => revealed.push(true) });
  return { props, actual, sent, canceled, revealed, atEnd };
}
const event = overrides => ({ key: 'Enter', shiftKey: false, isComposing: false, keyCode: 13,
  prevented: false, preventDefault() { this.prevented = true; }, ...overrides });

test('one send button and no sending-mode component remain in the actual Composer tree', () => {
  assert.equal(elements.filter(node => attribute(node, 'class')?.value === 'composer-round composer-send').length, 1);
  assert.equal(elements.some(node => node.openingElement.name.name === 'SendActions'), false);
  assert.equal(tree.program.body.some(node => node.type === 'ImportDeclaration' && node.source.value === './SendActions'), false);
});

test('button and Enter use the same no-argument send callback and preserve reply/focus behavior', () => {
  for (const kind of ['click', 'keydown']) {
    const f = fixture(), key = event();
    f.actual[kind](key);
    assert.deepEqual(f.sent, [[]]);
    assert.deepEqual(f.atEnd, [true]);
    assert.equal(f.revealed.length, 1);
    assert.deepEqual(JSON.parse(JSON.stringify(f.actual.focusAnchor())), { x: 42, y: 18 });
    if (kind === 'keydown') assert.equal(key.prevented, true);
  }
});

test('IME Enter, legacy composing code and Shift+Enter do not submit or suppress input', () => {
  for (const change of [{ isComposing: true }, { keyCode: 229 }, { shiftKey: true }, { key: 'a' }]) {
    const f = fixture(), key = event(change);
    f.actual.keydown(key);
    assert.equal(f.sent.length, 0);
    assert.equal(key.prevented, false);
  }
});

test('running button stops the current run while Enter never sends or stops it', () => {
  const f = fixture({ scene: { run: { status: 'running' } } });
  assert.equal(f.actual.disabled(), false);
  f.actual.keydown(event());
  assert.equal(f.sent.length, 0);
  assert.equal(f.canceled.length, 0);
  f.actual.click();
  assert.equal(f.canceled.length, 1);
  assert.equal(f.sent.length, 0);
});

test('sending, closing and empty input block both submit paths without changing focus/reply state', () => {
  for (const change of [{ sending: true }, { closing: true }, { draft: ' \n\t ' }]) {
    const f = fixture(change);
    assert.equal(f.actual.disabled(), true);
    f.actual.keydown(event());
    f.actual.click();
    assert.equal(f.sent.length, 0);
    assert.equal(f.revealed.length, 0);
    assert.equal(f.atEnd.length, 0);
    assert.equal(f.actual.focusAnchor(), undefined);
  }
  assert.equal(fixture({ closing: true, scene: { run: { status: 'running' } } }).actual.disabled(), true);
});

test('attachment-only input still submits through the single entry', () => {
  for (const kind of ['click', 'keydown']) {
    const f = fixture({ draft: ' ', refs: [{ kind: 'region', id: 'synthetic-region' }] });
    assert.equal(f.actual.disabled(), false);
    f.actual[kind](event());
    assert.deepEqual(f.sent, [[]]);
  }
});

test('a pending send blocks subsequent keyboard or click events using current props', () => {
  const f = fixture();
  f.props.onSend = (...args) => { f.sent.push(args); f.props.sending = true; };
  f.actual.keydown(event());
  f.actual.click();
  f.actual.keydown(event());
  assert.deepEqual(f.sent, [[]]);
  f.props.sending = false;
  f.props.draft = '下一条';
  f.actual.click();
  assert.deepEqual(f.sent, [[], []]);
});
