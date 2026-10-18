// SPDX-License-Identifier: MPL-2.0
// Render the actual production passive subtree; no desktop, IPC or user data.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import postcss from 'postcss';
import * as solid from 'solid-js';
import * as web from 'solid-js/web';

const raw = file => readFile(new URL(file, import.meta.url), 'utf8');
const plain = source => stripTypeScriptTypes(source, { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const source = await raw('./components/SpaceCanvas.tsx');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const elements = [];
const visit = (node, ancestors = []) => {
  if (!node || typeof node !== 'object') return;
  if (node.type === 'JSXElement') elements.push({ node, ancestors });
  for (const [key, value] of Object.entries(node)) {
    if (key === 'loc') continue;
    const next = [...ancestors, node];
    if (Array.isArray(value)) value.forEach(child => visit(child, next));
    else if (value && typeof value === 'object') visit(value, next);
  }
};
visit(ast);
const className = node => node.openingElement.attributes.find(attribute => attribute.name?.name === 'class')?.value?.value;
const passive = elements.find(entry => className(entry.node) === 'capture-saved-drawings');
assert.ok(passive, 'Saved ink needs an independent root paint layer');

const modules = new Map();
async function load(url) {
  if (modules.has(url.href)) return modules.get(url.href);
  if (url.pathname.endsWith('.css')) return synthetic({});
  if (url.pathname.endsWith('/bridge.ts')) return synthetic({ getMosaicPreview: async () => { throw Error('No fixture IPC'); }, getHighlightPreview: async () => { throw Error('No fixture IPC'); } });
  if (url.pathname.endsWith('/RichDrawingShape.tsx')) return synthetic({ default: () => '' });
  const text = await readFile(url, 'utf8');
  const code = url.pathname.endsWith('.tsx') ? transformSync(text, { filename: 'capture-stack.tsx', parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'ssr', hydratable: false }]], configFile: false, babelrc: false }).code : text;
  const module = new SourceTextModule(plain(code)); modules.set(url.href, module);
  await module.link(name => {
    if (name === 'solid-js') return synthetic(solid);
    if (name === 'solid-js/web') return synthetic(web);
    const target = new URL(name, url);
    if (!/\.(?:ts|tsx|css)$/.test(target.pathname)) target.pathname += target.pathname.endsWith('/RichDrawingShape') ? '.tsx' : '.ts';
    return load(target);
  });
  await module.evaluate(); return module;
}
const drawing = await load(new URL('./components/DrawingLayer.tsx', import.meta.url));
const projection = (await load(new URL('./region-image.ts', import.meta.url))).namespace.regionImageProjection;
const isolated = `import { For, createMemo } from 'solid-js'; import DrawingLayer from './DrawingLayer'; export default function saved(props,regionById,imageProjection,drawingSession){
  const regionIds = createMemo(() => props.scene.regions.map(region => region.id));
  const savedRegions = createMemo(() => new Map(props.scene.regions.map(region => [region.id, region])));
  return ${source.slice(passive.node.start, passive.node.end)};}`;
const compiled = transformSync(isolated, { filename: 'actual-passive-subtree.tsx', parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'ssr', hydratable: false }]], configFile: false, babelrc: false }).code;
const renderModule = new SourceTextModule(plain(compiled));
await renderModule.link(name => name === 'solid-js' ? synthetic(solid) : name === 'solid-js/web' ? synthetic(web) : drawing);
await renderModule.evaluate();
const background = { id: 'synthetic-background', kind: 'image', width: 1280, height: 800 };
const pen = { id: 'cross-pin', kind: 'line', color: '#EE4848', strokeWidth: 6, points: [{ x: 200, y: 300 }, { x: 900, y: 300 }] };
const region = { id: 'roi', x: 120, y: 160, width: 900, height: 500, drawingRevision: 1, drawings: [pen] };
function html(value = region, editing, overlay = value) {
  const props = { scene: { id: 'scene', background, regions: [value] } };
  return web.renderToString(() => renderModule.namespace.default(props, () => overlay, current => projection(current, { x: current.x * .5 + 20, y: current.y * .5 + 40, width: current.width * .5, height: current.height * .5 }, background, .5), () => editing ? { regionId: editing } : undefined));
}
test('saved drawings render outside selection stacking contexts and retain the exact original strokes after Done', () => {
  const ancestors = passive.ancestors.filter(node => node.type === 'JSXElement');
  assert.equal(ancestors.at(-1).openingElement.name.name, 'main');
  assert.equal(ancestors.some(node => className(node) === 'capture-region'), false);
  const allDrawings = elements.filter(entry => entry.node.openingElement.name.name === 'DrawingLayer');
  assert.equal(allDrawings.length, 1, 'Do not duplicate SVGs or their native preview readers');
  const saved = html();
  assert.ok(saved.includes('left:80px;top:120px;width:450px;height:250px'));
  assert.ok(saved.includes('viewBox="120 160 900 500"'));
  assert.ok(saved.includes('data-drawing-id="cross-pin"'));
  assert.ok(saved.includes('x1="200" y1="300" x2="900" y2="300"'));
  assert.ok(saved.includes('visibility:visible'));
  assert.equal((saved.match(/data-drawing-id="cross-pin"/g) ?? []).length, 1);
  assert.ok(html(region, 'roi').includes('visibility:hidden'));
  assert.ok(html(region, 'another-region').includes('visibility:visible'));
});
test('saved paint keeps geometry overlays and long-image letterboxing in the same projection as the editor', () => {
  const moved = { ...region, x: 180, y: 220, width: 800, height: 400 };
  const ordinary = html(region, undefined, moved);
  assert.ok(ordinary.includes('left:110px;top:150px;width:400px;height:200px'));
  assert.ok(ordinary.includes('viewBox="180 220 800 400"'));
  const long = { ...region, imageOverride: { id: 'long', kind: 'image', width: 400, height: 1200 } };
  const fitted = projection(long, { x: 80, y: 120, width: 450, height: 250 }, background, .5);
  const saved = html(long);
  assert.ok(saved.includes(`left:${fitted.box.x}px;top:${fitted.box.y}px;width:${fitted.box.width}px;height:${fitted.box.height}px`));
  assert.ok(saved.includes('viewBox="0 0 400 1200"'));
  assert.equal(html({ ...region, drawings: [] }).includes('<svg'), false);
});
test('the passive paint plane sits above pin pixels and below authoring/toolbars without intercepting their hit tests', async () => {
  const rules = new Map();
  for (const name of ['./components/capture.css', './components/drawing.css', './components/pin-objects.css']) {
    postcss.parse(await raw(name)).walkRules(rule => {
      const declaration = rules.get(rule.selector) ?? {};
      rule.walkDecls(value => { declaration[value.prop] = value.value; }); rules.set(rule.selector, declaration);
    });
  }
  const passiveStyle = rules.get('.capture-saved-drawings');
  assert.equal(passiveStyle.position, 'absolute'); assert.equal(passiveStyle.inset, '0');
  assert.equal(passiveStyle['pointer-events'], 'none');
  assert.equal(rules.get('.drawing-layer')['pointer-events'], 'none');
  assert.equal(rules.get('.capture-region-content')['pointer-events'], 'none');
  const pin = Number(rules.get('.space-pin-layer')['z-index']), ink = Number(passiveStyle['z-index']);
  assert.ok(Number(rules.get('.capture-region,.capture-selection')['z-index']) < pin);
  assert.ok(pin < ink); assert.ok(ink < Number(rules.get('.drawing-editor')['z-index']));
  assert.ok(ink < Number(rules.get('.capture-toolbar')['z-index']));
  assert.equal(rules.get('.space-pin-object')['pointer-events'], 'auto');
  assert.equal(rules.get('.capture-saved-drawings .space-pin-object'), undefined);
});
