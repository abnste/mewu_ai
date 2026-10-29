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
const prosemirror = new Map(await Promise.all(['prosemirror-model','prosemirror-state','prosemirror-view','prosemirror-commands','prosemirror-keymap','prosemirror-history'].map(async name=>[name,await import(name)])));

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
    if(prosemirror.has(name))return synthetic(prosemirror.get(name));
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

test('the real live-render port keeps literal run color and weight after editing and replacing the saved drawing',async()=>{
  const pointer=(await load(new URL('./components/drawing-pointer.ts',import.meta.url))).namespace;
  let current={...text,text:'Red Blue',textRuns:[{text:'Red ',color:'#EE4848',textStyle:{}},{text:'Blue',color:'#348BF1',textStyle:{bold:true}}]};
  const live=pointer.liveDrawing(current.id,()=>current);
  for(const color of ['#348BF1','#112233']){
    current={...current,textRuns:current.textRuns.map((run,index)=>index?{...run,color}:run)};
    const html=web.renderToString(()=>drawing.DrawingShape({drawing:live}));
    assert.ok(html.includes(`fill="${color}" font-weight="700"`),html);
    assert.ok(html.includes('fill="#EE4848" font-weight="400"'),html);
    assert.equal((html.match(/<text /g)??[]).length,2);
  }
});

test('video run wire roundtrips literal fragments and rejects malformed, partial or split-Unicode spans',()=>{
  const value={...text,text:'中文 😀',textRuns:[{text:'中文 ',color:'#EE4848',textStyle:{}},{text:'😀',color:'#348BF1',textStyle:{bold:true}}]};
  const source=wire.drawingToVideoSource(value);
  assert.deepEqual(source.content.textRuns,value.textRuns);assert.notEqual(source.content.textRuns,value.textRuns);
  assert.ok(wire.validVideoSourceDrawing(source));assert.ok(video.validateVideoTextContent(source.content));
  for(const textRuns of [[],[{text:'中文'}],[{text:'错误'}],[{text:'中文 😀',color:'red'}],[{text:'中文 😀',textStyle:{css:'bad'}}],[{text:'中文 \uD83D'},{text:'\uDE00'}]]){
    const invalid={...source,content:{...source.content,textRuns}};
    assert.equal(wire.validVideoSourceDrawing(invalid),false);assert.equal(video.validateVideoTextContent(invalid.content),false);
  }
});

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

test('inline editor is transparent HTML text rather than a native bordered textarea', async () => {
  const fonts = [];
  const previous = globalThis.document;
  globalThis.document = { createElement: () => ({ getContext: () => ({ set font(value) { fonts.push(value); }, measureText(value) { return { width: value.length * 20, fontBoundingBoxAscent: 30, fontBoundingBoxDescent: 10 }; } }) }) };
  try {
    const Inline = (await load(new URL('./components/InlineDrawingText.tsx', import.meta.url))).namespace.default;
    const html = web.renderToString(() => Inline({ drawing: { ...text, textStyle: { bold: true, italic: true, underline: true, strikethrough: true } }, disabled: false, ref() {}, onInput() {}, onConfirm() {}, onBlur() {} }));
    assert.ok(fonts.includes('normal 400 40px "Segoe UI", "Microsoft YaHei", sans-serif'));
    assert.ok(fonts.every(font => /^normal (400|700) 40px /.test(font)));
    assert.equal((html.match(/<textarea/g) ?? []).length, 0);
    assert.ok(html.includes('class="drawing-rich-text"'));
    const css=await readFile(new URL('./components/drawing.css',import.meta.url),'utf8');
    assert.match(css,/\.drawing-rich-text-content[^}]*border:\s*0\s*!important/s);
    assert.match(css,/\.drawing-rich-text-content[^}]*box-shadow:\s*none\s*!important/s);
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

test('video text boxes preserve inner dimensions and restore native four-pixel inset on resize without moving stored origin', async () => {
  const source = await readFile(new URL('./components/VideoDrawingEditor.tsx', import.meta.url), 'utf8');
  const body = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] }).program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body.body;
  const projected = body.find(node => node.type === 'VariableDeclaration' && node.declarations.some(entry => entry.id.name === 'projected'));
  const content = { version: 1, color: '#EE4848', fontSize: 28, text: '中文 e\u0301', textStyle: { bold: true, underline: true }, textBox: { width: 150, height: 20 } };
  const context = createContext({ structuredClone, cacheVersion() {}, object: () => ({ primitive: { kind: 'text', topLeft: { x: 40, y: 20 }, layout: 'ref' } }), texts: new Map([['text', { reference: 'ref', content }]]), sameVideoDrawingValue: (a, b) => JSON.stringify(a) === JSON.stringify(b) });
  runInContext(`${plain(source.slice(projected.start, projected.end))}\nglobalThis.result = projected('text');`, context);
  assert.deepEqual(JSON.parse(JSON.stringify(context.result.points)), [{ x: 44, y: 24 }, { x: 194, y: 44 }]);
  const roundTrip = wire.drawingToVideoSource(context.result, 4);
  assert.deepEqual(JSON.parse(JSON.stringify(roundTrip)), { kind: 'text', topLeft: { x: 40, y: 20 }, content });
  const resized = wire.drawingToVideoSource({ ...context.result, points: [{ x: 44, y: 24 }, { x: 244, y: 124 }] }, 4);
  assert.deepEqual(JSON.parse(JSON.stringify(resized.topLeft)), { x: 40, y: 20 });
  assert.deepEqual(JSON.parse(JSON.stringify(resized.content.textBox)), { width: 200, height: 100 });
  assert.ok(video.validateVideoTextContent(resized.content)); assert.ok(wire.validVideoSourceDrawing(resized));
  for (const textBox of [null, [], { width: 0, height: 10 }, { width: 10, height: -1 }, { width: 6001, height: 20 }, { width: 20 }, { width: 20, height: 10, css: 'url()' }]) {
    assert.equal(video.validateVideoTextContent({ ...content, textBox }), false);
    assert.equal(wire.validVideoSourceDrawing({ ...resized, content: { ...content, textBox } }), false);
  }
});

test('real text layout and SVG expand video TABs before wrapping without rewriting source content', () => {
  const value = { ...text, fontSize: 24, text: 'A\tB', points: [{ x: 40, y: 20 }, { x: 41, y: 21 }] };
  const layout = geometry.drawingTextLayout(value);
  assert.deepEqual(JSON.parse(JSON.stringify(layout.lines)), ['A', ' ', ' ', ' ', ' ', 'B']);
  assert.equal(layout.height, 6 * 24 * 1.25);
  const html = web.renderToString(() => drawing.DrawingShape({ drawing: value }));
  assert.equal((html.match(/<text /g) ?? []).length, 6);
  for (const baseline of [44, 74, 104, 134, 164, 194]) assert.ok(html.includes(`y="${baseline}"`));
  assert.equal(value.text, 'A\tB');
  assert.equal(wire.drawingToVideoSource(value).content.text, 'A\tB');
});

test('actual layout shares whole-line search for contextual nonmonotonic advances without losing graphemes', () => {
  // A real Segoe UI 28px prefix profile observed with GDI+ typographic metrics.
  // Inject it into the production layout; this test does not claim native/Canvas
  // pixel equality. Native wrap_lines uses the same candidate search order.
  const advances = new Map([['ا', 7.013672], ['اس', 39.744141], ['اسل', 49.095703], ['اسلا', 46.757812]]), probes = [];
  const value = { ...text, fontSize: 28, text: 'اسلا', points: [{ x: 40, y: 20 }, { x: 88, y: 60 }] };
  const layout = geometry.drawingTextLayout(value, part => { probes.push(part); assert.ok(advances.has(part)); return advances.get(part); });
  assert.deepEqual(JSON.parse(JSON.stringify(layout.lines)), ['اسلا']);
  assert.deepEqual(probes, ['اس', 'اسلا']);
  assert.equal(layout.contentWidth, advances.get('اسلا')); assert.ok(layout.contentWidth <= 48);
  assert.equal(layout.lines.join(''), value.text);
  for (const [literal, width, expected] of [['iiii', 4, ['iiii']], ['WWWW', 4, ['W', 'W', 'W', 'W']], ['中文中文', 4, ['中文', '中文']]]) {
    const drawing = { ...value, text: literal, points: [{ x: 40, y: 20 }, { x: 40 + width, y: 60 }] };
    const result = geometry.drawingTextLayout(drawing, part => [...part].reduce((sum, character) => sum + (character === 'i' ? 1 : character === 'W' ? 4 : 2), 0));
    assert.deepEqual(JSON.parse(JSON.stringify(result.lines)), expected); assert.equal(result.lines.join(''), literal);
  }
});
