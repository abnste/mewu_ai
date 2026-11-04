// Actual component handlers and Solid lifecycle, with a synthetic display and sampler.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
import test from 'node:test';
import * as solid from '../../../node_modules/solid-js/dist/solid.js';

const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const raw = name => readFile(new URL(name, import.meta.url), 'utf8');
const core = new SourceTextModule(stripTypeScriptTypes(await raw('./pointer-inspector.ts'), { mode: 'transform' }));
await core.link(() => { throw Error('Unexpected dependency'); }); await core.evaluate();
const source = await raw('./components/PointerInspector.tsx');
const ast = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
const body = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration.body;
const output = body.body.at(-1);
assert.equal(output.type, 'ReturnStatement');
// Only JSX presentation is omitted; all event handlers and effects run unchanged.
const component = new SourceTextModule(stripTypeScriptTypes(source.slice(0, output.start) + 'return null;' + source.slice(output.end), { mode: 'transform' }));
const samples = [], copies = [];
await component.link(name => name === 'solid-js' ? synthetic(solid) : name.endsWith('/pointer-inspector') ? synthetic(core.namespace)
  : name.endsWith('/pointer-bridge') ? synthetic({ getPointerSample: async request => { samples.push(request); return { ...request, hex: '#1234AB', dataUrl: 'data:image/png;base64,AAAA' }; } })
  : name.endsWith('/clipboard-text') ? synthetic({ writeClipboardText: async value => { copies.push(value); } })
  : name.endsWith('.css') ? synthetic({}) : (() => { throw Error(`Unexpected import ${name}`); })());
await component.evaluate();

test('Actual magnifier handlers batch hit testing, flush exact copy, and cancel on blur, source change and disposal', async () => {
  const prior = new Map(), frames = new Map(), window = new EventTarget(), document = new EventTarget();
  let frameId = 0, hitTests = 0, handle, dispose, disabled;
  document.documentElement = new EventTarget(); document.body = {}; document.hidden = false;
  document.hasFocus = () => true; document.querySelector = () => null;
  document.elementFromPoint = () => { hitTests++; return { closest: selector => selector === '.capture-surface' ? {} : null }; };
  const globals = { window, document, innerWidth: 1280, innerHeight: 720,
    MutationObserver: class { observe() {} disconnect() {} },
    requestAnimationFrame: callback => { const id = ++frameId; frames.set(id, callback); return id; },
    cancelAnimationFrame: id => frames.delete(id) };
  for (const [key, value] of Object.entries(globals)) { prior.set(key, Object.getOwnPropertyDescriptor(globalThis, key)); Object.defineProperty(globalThis, key, { value, configurable: true }); }
  const move = x => window.dispatchEvent(Object.assign(new Event('pointermove'), { clientX: x, clientY: 20 }));
  const tick = () => new Promise(resolve => setImmediate(resolve));
  try {
    solid.createRoot(cleanup => {
      dispose = cleanup; const [off, setOff] = solid.createSignal(false); disabled = setOff;
      component.namespace.default({ sceneId: 'scene', background: { id: 'background', width: 1280, height: 720 }, imageBox: { x: 0, y: 0, width: 1280, height: 720 },
        get disabled() { return off(); }, blockedAt: () => false, onReady: value => { handle = value; }, onError: error => { throw Error(error); } });
    });
    for (let x = 1; x <= 500; x++) move(x);
    assert.equal(hitTests, 0); assert.equal(samples.length, 0); assert.equal(frames.size, 1);
    [...frames.values()][0](); await tick();
    assert.equal(hitTests, 1); assert.equal(samples.length, 1); assert.equal(samples[0].x, 500);
    move(650); assert.equal(handle.copy(), true); await tick();
    assert.equal(samples.at(-1).x, 650); assert.deepEqual(copies, ['#1234AB']); assert.equal(frames.size, 0);
    move(700); window.dispatchEvent(new Event('blur')); assert.equal(frames.size, 0); assert.equal(handle.copy(), false);
    move(800); disabled(true); assert.equal(frames.size, 0); assert.equal(handle.copy(), false);
    disabled(false); move(900); dispose(); assert.equal(handle, undefined); assert.equal(frames.size, 0);
    move(950); await tick(); assert.equal(samples.length, 2);
  } finally {
    dispose?.();
    for (const [key, descriptor] of prior) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete globalThis[key]; }
  }
});
