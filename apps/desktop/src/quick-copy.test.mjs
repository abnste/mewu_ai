// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { createContext, runInContext } from 'node:vm';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
const source = await readFile(new URL('./components/OcrTextLayer.tsx', import.meta.url), 'utf8');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const component = ast.program.body.find(n => n.type === 'ExportDefaultDeclaration').declaration;
const methods = component.body.body.filter(n => n.type === 'FunctionDeclaration' && ['copy', 'clipboard'].includes(n.id.name));
const code = methods.map(n => stripTypeScriptTypes(source.slice(n.start, n.end), { mode: 'transform' })).join('\n');
test('actual OCR copy closes only its current document after clipboard success', async () => {
  for (const state of ['current', 'disposed', 'changed', 'inactive', 'failed']) {
    let settle, copied = 0, errors = 0;
    const done = new Promise((resolve, reject) => { settle = state === 'failed' ? () => reject(Error('clipboard denied')) : resolve; });
    const values = [], context = createContext({ disposed: false, key: () => 'document-one', selectedText: () => 'MEWU OCR', layout: () => ({ text: 'MEWU OCR' }), setMenu() {}, navigator: { clipboard: { writeText: async text => { values.push(text); await done; } } }, props: { active: true, onCopied: () => copied++, onError: () => errors++ } });
    runInContext(code, context);
    const copying = context.copy(false);
    if (state === 'disposed') context.disposed = true;
    if (state === 'changed') context.key = () => 'document-two';
    if (state === 'inactive') context.props.active = false;
    settle(); await copying;
    assert.deepEqual(values, ['MEWU OCR']);
    assert.equal(copied, state === 'current' ? 1 : 0);
    assert.equal(errors, state === 'failed' ? 1 : 0);
  }
});
test('native browser copy event never closes a successor from its delayed callback', () => {
  let callback, copied = 0;
  const context = createContext({ disposed: false, key: () => 'one', selectedText: () => 'TEXT', setTimeout: fn => { callback = fn; }, props: { active: true, onCopied: () => copied++ } });
  runInContext(code, context);
  const values = []; const event = { stopPropagation() {}, preventDefault() {}, clipboardData: { setData: (...args) => values.push(args) } };
  context.clipboard(event); context.key = () => 'two'; callback();
  assert.deepEqual(values, [['text/plain', 'TEXT']]); assert.equal(copied, 0);
  context.clipboard(event); callback(); assert.equal(copied, 1);
});
test('actual quick OCR Show remains mounted when a region has been selected', async () => {
  const canvas = await readFile(new URL('./components/SpaceCanvas.tsx', import.meta.url), 'utf8');
  const tree = parse(canvas, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  let condition;
  const visit = node => {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'JSXElement' && node.openingElement.name.name === 'Show' && node.children.some(child => canvas.slice(child.start, child.end).includes('class="quick-ocr"'))) {
      const attr = node.openingElement.attributes.find(a => a.name?.name === 'when');
      if (attr && canvas.slice(attr.start, attr.end).includes('props.quickText')) condition = attr.value.expression;
    }
    for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
  }; visit(tree);
  assert.ok(condition);
  const props = { quickText: { lines: [{}] }, scene: { regions: [{ id: 'selected-region' }] } };
  const context = createContext({ props, drawingSession: () => false, imageDrawing: () => false });
  assert.equal(runInContext(canvas.slice(condition.start, condition.end), context), true);
  context.drawingSession = () => true; assert.equal(runInContext(canvas.slice(condition.start, condition.end), context), false);
});
