// SPDX-License-Identifier: MPL-2.0
// Actual ProseMirror transactions/history, not a substitute editor or IPC mock.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { EditorState, TextSelection } from 'prosemirror-state';
import { undo, redo } from 'prosemirror-history';

const source = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./drawing-text-model.ts', import.meta.url), 'utf8'), { mode: 'transform' }));
await source.link(async name => {
  const exports = await import(name);
  return new SyntheticModule(Object.keys(exports), function () { for (const [key, value] of Object.entries(exports)) this.setExport(key, value); });
});
await source.evaluate();
const model = source.namespace;
const literal = { text: '中文 hello 👨‍👩‍👧', color: '#EE4848' };
const plain = value => JSON.parse(JSON.stringify(value));
const editor = value => EditorState.create({ doc: model.textDocument(value), plugins: model.drawingTextPlugins(() => {}) });
const select = (state, from, to) => state.apply(state.tr.setSelection(TextSelection.create(state.doc, from, to)));
const read = state => plain(model.readTextDocument(state.doc, literal));

test('a literal legacy document roundtrips without introducing runs or interpreting markup', () => {
  for (const text of [literal.text, '<svg onload="bad"> & 中文\n\nend\n', 'e\u0301 ❤️ 👨‍👩‍👧']) {
    const value = { ...literal, text, textStyle: { bold: true } };
    assert.deepEqual(plain(model.readTextDocument(model.textDocument(value), value)), { text });
  }
});

test('format only selected characters; preserve neighboring styles and text through undo/redo', () => {
  let state = editor(literal);
  state = select(state, 4, 9); // "hello", leaving the CJK and emoji alone.
  state = state.apply(model.formatTextSelection(state, { color: '#3366CC', textStyle: { bold: true } }));
  const expected = { text: literal.text, textRuns: [
    { text: '中文 ', color: '#EE4848', textStyle: {} },
    { text: 'hello', color: '#3366CC', textStyle: { bold: true } },
    { text: ' 👨‍👩‍👧', color: '#EE4848', textStyle: {} },
  ] };
  assert.deepEqual(read(state), expected);
  assert.deepEqual(plain(model.selectionTextStyle(state, literal.color)), { color: '#3366CC', textStyle: { bold: true } });
  const update = transaction => { state = state.apply(transaction); };
  assert.ok(undo(state, update)); assert.deepEqual(read(state), { text: literal.text });
  assert.ok(redo(state, update)); assert.deepEqual(read(state), expected);
  const restored = editor({ ...literal, ...expected });
  assert.deepEqual(read(restored), expected);
});

test('mixed selection toggles one effect without clearing unrelated italic or color', () => {
  let state = editor({ ...literal, text: 'ABCD', textRuns: [
    { text: 'AB', color: '#112233', textStyle: { italic: true } },
    { text: 'CD', color: '#AA2233', textStyle: { underline: true } },
  ] });
  state = select(state, 1, 5);
  state = state.apply(model.formatTextSelection(state, { textStyle: { bold: true } }));
  assert.deepEqual(read(state).textRuns, [
    { text: 'AB', color: '#112233', textStyle: { bold: true, italic: true } },
    { text: 'CD', color: '#AA2233', textStyle: { bold: true, underline: true } },
  ]);
  state = state.apply(model.formatTextSelection(state, { textStyle: {} }));
  assert.deepEqual(read(state).textRuns.map(run => run.textStyle), [{ italic: true }, { underline: true }]);
});

test('empty selection affects newly typed text instead of recoloring the document', () => {
  let state = select(editor({ ...literal, text: 'AB' }), 2, 2);
  state = state.apply(model.formatTextSelection(state, { color: '#0000FF', textStyle: { italic: true, strikethrough: true } }));
  state = state.apply(state.tr.insertText('中'));
  assert.deepEqual(read(state), { text: 'A中B', textRuns: [
    { text: 'A', color: '#EE4848', textStyle: {} },
    { text: '中', color: '#0000FF', textStyle: { italic: true, strikethrough: true } },
    { text: 'B', color: '#EE4848', textStyle: {} },
  ] });
});

test('plain multiline paste, replacement and undo preserve literal source and ignore HTML', () => {
  let state = select(editor({ ...literal, text: 'AB' }), 2, 2);
  state = state.apply(state.tr.replaceSelection(model.plainTextPaste('中\r\n<svg>\t&', state.selection.$from.marks())));
  assert.equal(read(state).text, 'A中\n<svg>    &B');
  assert.ok(undo(state, transaction => { state = state.apply(transaction); }));
  assert.equal(read(state).text, 'AB');
});

test('over-limit edit is rejected without truncating or destroying accepted text', () => {
  const errors = [];
  let state = EditorState.create({ doc: model.textDocument({ ...literal, text: '中'.repeat(1999) }), plugins: model.drawingTextPlugins(message => errors.push(message)) });
  state = state.apply(state.tr.insertText('好', 2000));
  assert.equal(read(state).text.length, 2000);
  const before = state.doc;
  state = state.apply(state.tr.insertText('坏', 2001));
  assert.ok(state.doc.eq(before)); assert.equal(errors.length, 1);
});
