// SPDX-License-Identifier: MPL-2.0
// Actual SpaceCanvas region JSX and pointer handlers, Solid's reactive runtime,
// synthetic DOM/persistence only. Counts are deterministic; timings are diagnostic.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile, writeFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { performance } from 'node:perf_hooks';
import { parse } from '@babel/parser';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const plain = source => stripTypeScriptTypes(source, { mode: 'transform' });
const source = process.env.MEWU_CANVAS_PERF_SOURCE
  ? await readFile(process.env.MEWU_CANVAS_PERF_SOURCE, 'utf8') : await raw('./components/SpaceCanvas.tsx');
const nodes = [];
function visit(node) {
  if (!node || typeof node !== 'object') return;
  if (node.type) nodes.push(node);
  for (const [key, value] of Object.entries(node)) {
    if (key === 'loc') continue;
    if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
  }
}
visit(parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] }));
const slice = node => source.slice(node.start, node.end);
const declaration = name => nodes.find(node => node.type === 'VariableDeclarator' && node.id.name === name);
const variable = (name, instrument = false) => {
  const node = declaration(name);
  if (!node) return '';
  return instrument ? `const ${name}=(...args)=>{metrics.${name}++; return (${slice(node.init)})(...args);};` : `const ${slice(node)};`;
};
const handler = name => slice(nodes.find(node => node.type === 'FunctionDeclaration' && node.id.name === name));
const element = name => nodes.find(node => node.type === 'JSXElement' && node.openingElement.attributes.some(attribute => attribute.name?.name === 'class' && attribute.value?.value === name));
const mask = element('capture-mask'), passive = element('capture-saved-drawings'), region = element('capture-region');
const regionList = nodes.find(node => node.type === 'JSXElement' && node.openingElement.name.name === 'For' && node.start < region.start && node.end > region.end);
const harness = `import {createSignal,createMemo,Show,For,batch,untrack} from 'solid-js';
import {overlayRegion,geometryTarget,sameGeometry} from './region-geometry';
import {regionImageProjection} from './region-image';
import {DrawingInputPhase,drawingDiagnosticsEnabled,recordDrawingInput} from './drawing-diagnostics';
export default function Owner(props){
  const metrics=props.metrics,canvas=props.canvas;
  let disposed=false,interactionVersion=0,cancelGesture,newSelectionSource;
  const [activity,setActivity]=createSignal(false),[active,setActive]=createSignal(''),[activeItemId,setActiveItemId]=createSignal('');
  const drawingSession=()=>undefined,forceNew=()=>false,flashedRegion=()=>'',ocrPending=()=>undefined,ocrNotice=()=>undefined;
  const exporting=()=>false,settling=()=>false,extensionRunning=()=>false;
  const geometry=createMemo(()=>props.geometry),blocked=()=>false,ocrVisible=()=>false,translationVisible=()=>false;
  const selectionPreview=()=>undefined,queuedSelectionPreviews=()=>[],savedTranslation=()=>undefined,savedOcr=()=>undefined;
  const pendingSelectionOutlines=()=>[];
  const t=value=>value,assetUrl=value=>value.path,EDGES=['n','e','s','w'],CORNERS=['nw','ne','sw','se'];
  const TranslationLayer=()=>null,OcrTextLayer=()=>null,LoaderCircle=()=>null,X=()=>null;
  const DrawingLayer=layer=>{createMemo(()=>{layer.region;layer.sourceRegion;metrics.drawingReads++;});return null;};
  const clearPendingSelection=()=>{},clearSnapPreview=()=>{},hideToolbar=()=>{},endNudge=()=>{},deferGeometryPointer=()=>false;
  const ocrRequests={cancel(){}},candidateAt=()=>undefined,snapResize=value=>value,begin=()=>{},revealToolbar=()=>{};
  const clamp=(value,low,high)=>Math.max(low,Math.min(Math.max(low,high),value));
  ${['savedRegions','regionIds'].map(name=>variable(name)).join('\n')}
  ${variable('regionById',true)}
  ${variable('display',true)}
  ${variable('imageProjection',true)}
  ${variable('regions')}
  ${['setSelectionActivity','trackPointer','editRegion','presentPendingSelection'].map(handler).join('\n')}
  Object.assign(props.control,{regionById,regions,stop:()=>cancelGesture?.()});
  return <div>${slice(mask)}${slice(regionList)}${slice(passive)}</div>;
}`;
const compiled = plain(transformSync(harness, { filename: 'real-capture-region.tsx', babelrc: false, configFile: false,
  parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'universal', moduleName: 'renderer' }]] }).code);

async function fixture(count) {
  const metrics = { creates: 0, removes: 0, writes: 0, findComparisons: 0, regionById: 0, display: 0, imageProjection: 0, drawingReads: 0 };
  const listeners = new Map(), capture = new Set(), control = {};
  const canvas = { addEventListener(name, fn) { (listeners.get(name) ?? (listeners.set(name, new Set()), listeners.get(name))).add(fn); },
    removeEventListener: (name, fn) => listeners.get(name)?.delete(fn), setPointerCapture: id => capture.add(id),
    hasPointerCapture: id => capture.has(id), releasePointerCapture: id => capture.delete(id), focus() {} };
  const context = createContext({ console, queueMicrotask, setTimeout, clearTimeout,
    window: { getSelection: () => ({ removeAllRanges() {} }) } });
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, { context });
  const core = synthetic(solid), universal = new SourceTextModule(await raw('../../../node_modules/solid-js/universal/dist/universal.js'), { context });
  await universal.link(() => core); await universal.evaluate();
  const renderer = universal.namespace.createRenderer({
    createElement: type => { metrics.creates++; return { type, children: [], props: {}, parent: null }; },
    createTextNode: text => { metrics.creates++; return { text, children: [], parent: null }; },
    replaceText: (node, text) => { node.text = text; }, setProperty: (node, key, value) => { metrics.writes++; node.props[key] = value; },
    insertNode: (parent, node, anchor) => { if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1); const index = anchor ? parent.children.indexOf(anchor) : -1; parent.children.splice(index < 0 ? parent.children.length : index, 0, node); node.parent = parent; },
    removeNode: (parent, node) => { metrics.removes++; parent.children.splice(parent.children.indexOf(node), 1); node.parent = null; },
    isTextNode: node => 'text' in node, getParentNode: node => node.parent, getFirstChild: node => node.children[0], getNextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1],
  });
  const geometryModule = new SourceTextModule(plain(await raw('./region-geometry.ts')), { context });
  await geometryModule.link(() => { throw Error('unexpected import'); }); await geometryModule.evaluate();
  const imageModule = new SourceTextModule(plain(await raw('./region-image.ts')), { context });
  await imageModule.link(() => { throw Error('unexpected import'); }); await imageModule.evaluate();
  const diagnosticModule = new SourceTextModule(plain(await raw('./drawing-diagnostics.ts')), { context });
  await diagnosticModule.link(() => { throw Error('unexpected diagnostics import'); }); await diagnosticModule.evaluate();
  const owner = new SourceTextModule(compiled, { context });
  await owner.link(name => name === 'solid-js' ? core : name === 'renderer' ? synthetic(renderer) : name === './region-geometry' ? geometryModule : name === './drawing-diagnostics' ? diagnosticModule : imageModule); await owner.evaluate();
  const measureRegions = values => new Proxy(values, { get(target, key, receiver) {
    if (key !== 'find') return Reflect.get(target, key, receiver);
    return predicate => target.find((value, index) => { metrics.findComparisons++; return predicate(value, index, target); });
  } });
  const background = { id: 'background', kind: 'image', width: 1920, height: 1080 };
  const initial = { id: 'scene', background, regions: measureRegions(Array.from({ length: count }, (_, index) => ({
    id: `region-${index}`, x: 80 + index % 10 * 120, y: 80 + Math.floor(index / 10) * 100, width: 100, height: 75,
  }))), items: [], frozen: false, closed: false };
  const [scene, setScene] = solid.createSignal(initial), [view, setView] = solid.createSignal({pending:false,held:false});
  let target;
  const props = { get scene() { return scene(); }, get geometryView() { return view(); }, control, canvas, metrics,
    geometry: { x: 0, y: 0, width: 1920, height: 1080, scale: 1 }, maskOpacity: .4,
    geometryController: { beginPointer(value) { target = value; return true; }, previewPointer(geometry) { setView({pending:true,held:true,mode:'pointer',overlay:{...target,geometry}}); }, endPointer: async () => {} } };
  const root = renderer.createElement('root'), dispose = renderer.render(() => owner.namespace.default(props), root);
  const all = () => { const result = []; const visit = node => { result.push(node); node.children.forEach(visit); }; visit(root); return result; };
  const sections = () => all().filter(node => node.props?.class === 'capture-region');
  const cutouts = () => all().filter(node => node.type === 'rect' && node.props.fill === 'black');
  const pointer = (x, y) => ({button:0,pointerId:1,shiftKey:false,clientX:x,clientY:y,preventDefault(){},stopPropagation(){}});
  const reset = () => { for (const key of Object.keys(metrics)) metrics[key] = 0; };
  return { metrics, control, props, scene, setScene: value => setScene({...value,regions:measureRegions(value.regions)}), setView, sections, cutouts, reset,
    begin() { sections().at(-1).props.onPointerDown(pointer(100,100)); },
    move(x, y) { for (const listener of listeners.get('pointermove') ?? []) listener(pointer(x,y)); },
    dispose() { control.stop(); dispose(); } };
}

const results = [];
for (const count of [1,10,30,60]) test(`${process.env.MEWU_CANVAS_PERF_BASELINE ? 'baseline diagnostic' : 'regression'}: actual ${count}-region pointer move`, async () => {
  const f = await fixture(count);
  try {
    f.begin(); const originalMasks = [...f.cutouts()], sections = [...f.sections()];
    f.reset(); const frames = 120, start = performance.now();
    for (let frame = 0; frame < frames; frame++) f.move(110 + frame % 80,120 + frame % 70);
    const elapsedMs = performance.now() - start;
    const result = { count, frames, elapsedMs, averageMs: elapsedMs / frames, ...f.metrics };
    results.push(result);
    if (!process.env.MEWU_CANVAS_PERF_BASELINE) {
      assert.equal(f.metrics.findComparisons, 0, 'do not linearly search saved regions during drag');
      assert.equal(f.metrics.creates, 0, 'existing mask/region nodes must remain mounted');
      assert.equal(f.metrics.removes, 0);
      assert.deepEqual(f.cutouts(), originalMasks);
      assert.deepEqual(f.sections(), sections);
      assert.ok(f.metrics.imageProjection <= frames * 2, 'only the moved region needs image projection');
      assert.ok(f.metrics.writes <= frames * 24, 'unmoved boxes do not receive DOM geometry writes');
      assert.equal(f.cutouts().at(-1).props.x, f.control.regionById(`region-${count-1}`).x);
    }
  } finally { f.dispose(); }
});

test('region identity index cannot authorize geometry from another source and keeps matching overrides letterboxed', async () => {
  const f = await fixture(3);
  try {
    const region = f.scene().regions[1], before = f.sections()[1].props.style;
    for (const invalid of [{sceneId:'different'}, {backgroundId:'different'}, {sourceId:'different'}, {regionId:'absent'}]) {
      f.setView({pending:true,held:true,overlay:{sceneId:'scene',regionId:region.id,backgroundId:'background',sourceId:'background',geometry:{...region,x:700},...invalid}});
      assert.equal(f.control.regionById(region.id), region);
      assert.deepEqual(f.sections()[1].props.style,before);
    }
    const image = {...region,imageOverride:{id:'long-image',kind:'image',width:200,height:1000}};
    f.setScene({...f.scene(),regions:f.scene().regions.map(value=>value.id===image.id?image:value)});
    f.setView({pending:true,held:true,overlay:{sceneId:'scene',regionId:image.id,backgroundId:'background',sourceId:'background',geometry:{...image,x:700}}});
    assert.equal(f.control.regionById(image.id),image,'old background authority cannot move replacement pixels');
    f.setView({pending:true,held:true,overlay:{sceneId:'scene',regionId:image.id,backgroundId:'background',sourceId:'long-image',geometry:{...image,x:700}}});
    assert.equal(f.control.regionById(image.id).x,700);
    assert.equal(f.sections()[1].props.style.left,'700px');
    const content=f.sections()[1].children.find(node=>node.props?.class==='capture-region-content');
    assert.equal(content.props.style.width,'15px');
    assert.equal(content.props.style.height,'75px');
  } finally {f.dispose();}
});

test('removing regions or replacing the scene disposes the keyed views before a removed source is read', async () => {
  const f = await fixture(60);
  try {
    f.setScene({...f.scene(),regions:[f.scene().regions[0]]});
    assert.equal(f.sections().length,1);assert.equal(f.cutouts().length,1);
    f.setScene({...f.scene(),id:'replacement-scene',background:{...f.scene().background,id:'replacement-background'},regions:[]});
    assert.equal(f.sections().length,0);assert.equal(f.cutouts().length,0);
  } finally {f.dispose();}
});

test.after(async () => {
  if (process.env.MEWU_CANVAS_PERF_OUTPUT) await writeFile(process.env.MEWU_CANVAS_PERF_OUTPUT,JSON.stringify({kind:'actual-Solid-region-JSX-and-pointer-handlers-synthetic-renderer',results},null,2)+'\n');
});
