// SPDX-License-Identifier: MPL-2.0
// Production SVG/caret/wire execution only; native raster and durable DB have separate Rust tests.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext, runInContext } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from 'solid-js';
import * as web from 'solid-js/web';

const plain = source => stripTypeScriptTypes(source, { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const modules = new Map();
async function load(url) {
  if (modules.has(url.href)) return modules.get(url.href);
  if (url.pathname.endsWith('.css')) return synthetic({});
  if (url.pathname.endsWith('/bridge.ts')) return synthetic({ getMosaicPreview: async () => { throw Error('No fixture IPC'); }, getHighlightPreview: async () => { throw Error('No fixture IPC'); } });
  if (url.pathname.endsWith('/RichDrawingShape.tsx')) return synthetic({ default: () => '' });
  if (url.pathname.endsWith('/i18n.ts')) return synthetic({ t: value => value });
  const source = await readFile(url, 'utf8');
  const code = url.pathname.endsWith('.tsx') ? transformSync(source, { filename: 'text-style.tsx', parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'ssr', hydratable: false }]], configFile: false, babelrc: false }).code : source;
  const result = new SourceTextModule(plain(code)); modules.set(url.href, result);
  await result.link(name => {
    if (name === 'solid-js') return synthetic(solid);
    if (name === 'solid-js/web') return synthetic(web);
    const target = new URL(name, url);
    if (!/\.(?:ts|tsx|css)$/.test(target.pathname)) target.pathname += target.pathname.endsWith('/RichDrawingShape') ? '.tsx' : '.ts';
    return load(target);
  });
  await result.evaluate(); return result;
}
const drawing = (await load(new URL('./components/DrawingLayer.tsx', import.meta.url))).namespace;
const geometry = (await load(new URL('./components/drawing-geometry.ts', import.meta.url))).namespace;
const wire = (await load(new URL('./video-drawing-wire.ts', import.meta.url))).namespace;
const video = (await load(new URL('./video-manual-edit.ts', import.meta.url))).namespace;
const text = { id: 'text', kind: 'text', color: '#EE4848', strokeWidth: 2, fontSize: 40, points: [{ x: 40, y: 20 }], text: '中文 <&>\nAbc' };

test('actual production SVG combines all sixteen text styles with individually aligned CJK and Latin lines', () => {
  const rust = readFile(new URL('../src-tauri/src/drawing_render.rs', import.meta.url), 'utf8');
  assert.equal(geometry.DRAWING_TEXT_SKEW_DEGREES, -12);
  for (let flags = 0; flags < 16; flags++) {
    const style = { bold: !!(flags & 1), italic: !!(flags & 2), underline: !!(flags & 4), strikethrough: !!(flags & 8) };
    const html = web.renderToString(() => drawing.DrawingShape({ drawing: { ...text, textStyle: style } }));
    assert.equal((html.match(/<text /g) ?? []).length, 2);
    assert.ok(html.includes(`font-weight="${style.bold ? 700 : 400}"`));
    assert.ok(html.includes('font-style="normal"'), 'never double-italic an already oblique Latin face');
    for (const baseline of [60, 110]) {
      assert.ok(html.includes(`y="${baseline}"`));
      if (style.italic) assert.ok(html.includes(`transform="translate(0 ${baseline}) skewX(-12) translate(0 ${-baseline})"`));
    }
    if (!style.italic) assert.ok(!html.includes('skewX'));
    const decoration = [style.underline && 'underline', style.strikethrough && 'line-through'].filter(Boolean).join(' ') || 'none';
    assert.equal((html.match(new RegExp(`text-decoration:${decoration}`, 'g')) ?? []).length, 2);
    assert.ok(html.includes('中文 &lt;&amp;'));
    assert.ok(!html.includes('中文 <&>'), 'literal input must never become SVG markup');
  }
  return rust.then(source => assert.ok(source.includes(`const DRAWING_TEXT_SKEW_DEGREES: i32 = ${geometry.DRAWING_TEXT_SKEW_DEGREES};`)));
});

test('actual inline caret measures the same normal/bold advance without opening a second editor', async () => {
  const fonts = [];
  const previous = globalThis.document;
  globalThis.document = { createElement: () => ({ getContext: () => ({ set font(value) { fonts.push(value); }, measureText(value) { return { width: value.length * 20, fontBoundingBoxAscent: 30, fontBoundingBoxDescent: 10 }; } }) }) };
  try {
    const Inline = (await load(new URL('./components/InlineDrawingText.tsx', import.meta.url))).namespace.default;
    const html = web.renderToString(() => Inline({ drawing: { ...text, textStyle: { bold: true, italic: true, underline: true, strikethrough: true } }, disabled: false, ref() {}, onInput() {}, onConfirm() {}, onBlur() {} }));
    assert.deepEqual(fonts, ['normal 700 40px "Segoe UI", "Microsoft YaHei", sans-serif']);
    assert.equal((html.match(/<textarea/g) ?? []).length, 1);
    assert.ok(html.includes('font-weight:700;'));
    assert.ok(html.includes('font-style:normal;'));
    assert.ok(html.includes('aria-label="绘制文字"'));
  } finally { globalThis.document = previous; }
});

test('video text wire preserves all styles, isolates copies, keeps old payloads and rejects unknown CSS or nonboolean values', () => {
  const style = { bold: true, italic: true, underline: true, strikethrough: true };
  const source = wire.drawingToVideoSource({ ...text, textStyle: style });
  assert.deepEqual(source.content.textStyle, style);
  style.bold = false; assert.equal(source.content.textStyle.bold, true);
  assert.ok(wire.validVideoSourceDrawing(source)); assert.ok(video.validateVideoTextContent(source.content));
  const legacy = wire.drawingToVideoSource(text);
  assert.ok(!Object.hasOwn(legacy.content, 'textStyle'));
  assert.ok(wire.validVideoSourceDrawing(legacy)); assert.ok(video.validateVideoTextContent(legacy.content));
  for (const bad of [null, [], 'bold', { bold: 'true' }, { italic: 1 }, { fontFamily: 'https://example.invalid/font' }, { underline: true, unknown: false }]) {
    const invalid = { ...source, content: { ...source.content, textStyle: bad } };
    assert.equal(wire.validVideoSourceDrawing(invalid), false, JSON.stringify(bad));
    assert.equal(video.validateVideoTextContent(invalid.content), false, JSON.stringify(bad));
    assert.throws(() => wire.drawingToVideoSource({ ...text, textStyle: bad }));
  }
  assert.throws(() => wire.drawingToVideoSource({ id: 'line', kind: 'line', color: '#EE4848', strokeWidth: 2, points: [{ x: 1, y: 1 }, { x: 2, y: 2 }], textStyle: {} }));
});

test('actual video projection restores styles and never treats style-only changes as geometry-only movement', async () => {
  const source = await readFile(new URL('./components/VideoDrawingEditor.tsx', import.meta.url), 'utf8');
  const body = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] }).program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body;
  const declaration = name => { const node = body.find(node => node.type === 'VariableDeclaration' && node.declarations.some(entry => entry.id.name === name)); return plain(source.slice(node.start, node.end)); };
  const style = { bold: true, italic: true, underline: true, strikethrough: true };
  const context = createContext({ structuredClone, cacheVersion() {}, object: () => ({ primitive: { kind: 'text', topLeft: { x: 40, y: 20 }, layout: 'ref' } }), texts: new Map([['text', { reference: 'ref', content: { version: 1, color: '#EE4848', fontSize: 40, text: '中文', textStyle: style } }]]), sameVideoDrawingValue: (a, b) => JSON.stringify(a) === JSON.stringify(b) });
  runInContext(`${declaration('projected')}\n${declaration('isTranslation')}\nglobalThis.result = projected('text');globalThis.move = isTranslation;`, context);
  assert.deepEqual(context.result.textStyle, style);
  assert.notEqual(context.result.textStyle, style);
  const moved = { ...context.result, points: [{ x: 60, y: 40 }] };
  assert.equal(context.move(context.result, moved), true);
  assert.equal(context.move(context.result, { ...moved, textStyle: { ...style, bold: false } }), false);
});
