// node --experimental-vm-modules apps/desktop/src/drawing-layout.test.mjs
// Synthetic PNG headers/requests only; no native clipboard, model or user database.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { parse } from '@babel/parser';
const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const transform = source => stripTypeScriptTypes(source, { mode: 'transform' });
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
async function pure(path) { const mod = new SourceTextModule(transform(await raw(path))); await mod.link(() => { throw Error('unexpected dependency'); }); await mod.evaluate(); return mod.namespace; }
const preview = await pure('./drawing-layout-preview.ts'), domain = await pure('./drawing-document.ts'), geometry = await pure('./components/drawing-geometry.ts'), properties = await pure('./components/drawing-properties.ts');
const imageCode=await raw('./components/DrawingEditor.tsx'),imageAst=parse(imageCode,{sourceType:'module',plugins:['typescript','jsx']});
const imageBody=imageAst.program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration.body.body;
const imagePortCode=imageBody.filter(node=>node.type!=='ReturnStatement').map(node=>{let value=imageCode.slice(node.start,node.end);if(node.type==='VariableDeclaration'&&node.declarations[0].id.name==='port'){const render=node.declarations[0].init.properties.find(property=>property.key.name==='render').value;value=value.slice(0,render.start-node.start)+'()=>undefined'+value.slice(render.end-node.start);}return value;}).join('\n');
const imageMod=new SourceTextModule(transform(`import { drawingDraftKey,drawingPropertyCommand,richPreviewTarget,richSourceIdentity,usesDrawingDocument,richDocumentHistoryStep } from 'deps';const mosaicBlockSize=()=>8,getMosaicPreview=async()=>{};export function imagePort(props){${imagePortCode};return port;}`));await imageMod.link(()=>synthetic({...properties,...preview,...domain}));await imageMod.evaluate();
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const id = n => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const reference = { layoutId: id(4), layoutSha256: 'a'.repeat(64), rasterSha256: 'b'.repeat(64), kind: 'table', width: 1, height: 1 };
const target = { sceneId: id(1), regionId: id(2), drawingId: id(3), expectedRevision: 7, reference };
const png = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==';
const receipt = (value = target) => ({ ...structuredClone(value), dataUrl: png });
const drawing = { id: id(3), kind: 'rich', color: '#000000', strokeWidth: 1, points: [{ x: 20, y: 30 }, { x: 140, y: 110 }], rich: reference, origin: { runId: id(9), toolEventId: id(10), userMessageId: id(11), groupId: id(12), targetHandle: 'tile-1', manifestSha256: 'c'.repeat(64) } };
const region = { id: id(2), x: 10, y: 10, width: 300, height: 200, drawingRevision: 7, drawings: [drawing], drawingHistory: { undo: [], redo: [] } };
const asset = { id: id(20), name: 'synthetic.png', kind: 'image', path: 'assets/synthetic.png', width: 500, height: 400 };
const checks = [];
{
  assert.deepEqual(preview.validateLayoutPreview(receipt(), target), receipt());
  for (const bad of [
    { ...receipt(), sceneId: id(30) }, { ...receipt(), expectedRevision: 8 },
    { ...receipt(), reference: { ...reference, rasterSha256: 'c'.repeat(64) } },
    { ...receipt(), reference: { ...reference, extra: true } }, { ...receipt(), extra: true },
    { ...receipt(), dataUrl: 'https://example.test/image.png' }, { ...receipt(), dataUrl: 'data:image/svg+xml,<svg/>' },
  ]) assert.throws(() => preview.validateLayoutPreview(bad, target));
  const large = { ...target, reference: { ...reference, width: 6001 } }; assert.equal(preview.validLayoutTarget(large), false);
  const wrongSize = { ...target, reference: { ...reference, width: 2 } }; assert.throws(() => preview.validateLayoutPreview(receipt(wrongSize), wrongSize));
  assert.equal(preview.validRichReference({ ...reference, kind: 'formula' }), true);
  checks.push('Exact echoed target/ref and bounded PNG header; wrong source/revision/hash/dimensions/HTML/URL are rejected');
}
{
  const context = { sceneId: id(1), background: asset, region: { ...region, imageOverride: { ...asset, id: id(21), width: 200, height: 2200 } } };
  assert.deepEqual(preview.richPreviewTarget(context, drawing), target);
  const key = preview.richSourceIdentity(context);
  for (const changed of [ { ...context, sceneId: id(22) }, { ...context, background: { ...asset, path: 'assets/replaced.png' } }, { ...context, region: { ...context.region, imageOverride: { ...context.region.imageOverride, height: 2300 } } }, { ...context, region: { ...context.region, x: 0 } } ]) assert.notEqual(preview.richSourceIdentity(changed), key);
  assert.equal(preview.richPreviewTarget(context, { ...drawing, points: [{ x: 0, y: 0 }, { x: 120, y: 80 }] }).regionId, region.id);
  assert.notEqual(preview.layoutRequestKey(target, key), preview.layoutRequestKey({ ...target, reference: { ...reference, layoutSha256: 'c'.repeat(64) } }, key));
  checks.push('Actual region/source identities remain separate from long-image display geometry; full reference participates in cache keys');
}
{
  const a = deferred(), b = deferred(), published = [];
  const next = { ...target, reference: { ...reference, layoutId: id(31) } };
  const reader = new preview.DrawingLayoutReader(value => value.reference.layoutId === reference.layoutId ? a.promise : b.promise, value => published.push(value));
  reader.select(target, 'source A'); await tick(); reader.select(next, 'source B'); await tick();
  b.resolve(receipt(next)); await tick(); a.resolve(receipt()); await tick();
  assert.equal(published.at(-1).preview.reference.layoutId, id(31));
  assert.equal(published.filter(value => value.preview?.reference.layoutId === reference.layoutId).length, 0);
  reader.select(undefined); assert.deepEqual(published.at(-1), { loading: false });
  reader.select(target); reader.dispose(); await tick(); assert.equal(published.at(-1).preview, undefined);
  const failure = []; const sync = new preview.DrawingLayoutReader(() => { throw Error('synchronous fault'); }, value => failure.push(value)); sync.select(target); await tick(); assert.equal(failure.at(-1).error, 'synchronous fault');
  checks.push('Old layout/scene completion cannot repaint a successor, deleted object or disposed layer; synchronous reader errors remain visible errors');
}
{
  const cache = new preview.DrawingLayoutPreviewCache(2, 8), jobs = [], tasks = [];
  let live = 0, maximum = 0;
  for (let i = 0; i < 5; i++) {
    const value = { ...target, drawingId: id(40+i) }, gate = deferred(); jobs.push({ gate, value });
    tasks.push(cache.get(value, `source ${i}`, async () => { live++; maximum = Math.max(maximum, live); try { return await gate.promise; } finally { live--; } }));
  }
  await tick(); assert.equal(live, 2);
  for (const { gate, value } of jobs) { gate.resolve(receipt(value)); await tick(); assert.ok(live <= 2); }
  await Promise.all(tasks); assert.equal(maximum, 2);
  let calls = 0; const coalesced = new preview.DrawingLayoutPreviewCache(); const gate = deferred();
  const first = coalesced.get(target, 'same', () => { calls++; return gate.promise; });
  const second = coalesced.get(target, 'same', () => { calls++; return gate.promise; }); assert.equal(first, second);
  await tick(); gate.resolve(receipt()); await first; assert.equal(calls, 1);
  await coalesced.get(target, 'same', () => { throw Error('should be cached'); });
  const faulty = new preview.DrawingLayoutPreviewCache(); await assert.rejects(faulty.get(target, 'same', async () => { throw Error('read fault'); }));
  assert.deepEqual(await faulty.get(target, 'same', async () => receipt()), receipt());
  checks.push('One shared broker bounds real requests across readers, coalesces exact targets and never caches failed reads');
}
{
  const cache = new preview.DrawingLayoutPreviewCache(1, 1, 1, 1024), gate = deferred();
  const first = cache.get(target, 'one', () => gate.promise);
  await assert.rejects(cache.get({ ...target, drawingId: id(91) }, 'two', async () => receipt()), /繁忙/);
  gate.resolve(receipt()); await first; await tick();
  const other = { ...target, drawingId: id(92) }; await cache.get(other, 'two', async () => receipt(other));
  let refreshed = 0; await cache.get(target, 'one', async () => { refreshed++; return receipt(); }); assert.equal(refreshed, 1);
  checks.push('Pending admission and ready LRU are bounded independently; eviction never releases an unfinished read slot');
}
{
  assert.deepEqual(geometry.drawingBounds(drawing), { x: 20, y: 30, width: 120, height: 80 });
  const vector = { ...drawing, id: id(60), kind: 'line', rich: undefined }, mosaic = { ...vector, id: id(61), kind: 'mosaic' };
  assert.deepEqual(geometry.drawingOrder([vector, drawing, mosaic, { ...vector, id: id(62) }]).map(v => v.id), [id(61), id(60), id(3), id(62)]);
  const copy = properties.copyDrawing(drawing); copy.rich.layoutId = id(63); copy.origin.groupId = id(64); copy.points[0].x = 99;
  assert.equal(drawing.rich.layoutId, reference.layoutId); assert.equal(drawing.origin.groupId, id(12)); assert.equal(drawing.points[0].x, 20);
  assert.throws(() => properties.propertyDraft(drawing, 7));
  assert.throws(() => properties.changeProperty({ base: drawing, drawing, expectedRevision: 7, mode: 'style' }, { color: '#ff0000' }));
  const mixed = { batch: [{ index: 0, before: null, after: vector }, { index: 1, before: null, after: drawing }] };
  assert.equal(domain.richDocumentHistoryStep(mixed), true);
  assert.equal(domain.hasDrawingDocument({ ...region, drawings: [], drawingHistory: { undo: [], redo: [mixed] } }), true);
  assert.equal(domain.usesDrawingDocument(region, { type: 'add_drawing', drawing }), false);
  assert.equal(domain.usesDrawingDocument(region, { type: 'remove_drawing', drawingId: vector.id }), false);
  assert.equal(domain.usesDrawingDocument(region, { type: 'update_drawing', drawing }), true);
  assert.equal(domain.usesDrawingDocument({ ...region, drawingHistory: { undo: [mixed], redo: [] } }, { type: 'undo_drawing' }), true);
  checks.push('Rich boxes have exact hit bounds/layer order, detached immutable refs/provenance, no style draft; mixed history remains one document step');
}
{
  const vector = { ...drawing, id: id(65), kind: 'line', rich: undefined };
  const step = { index: 0, before: null, after: vector };
  const undoOnly = { ...region, drawings: [vector], drawingHistory: { undo: [step], redo: [] } };
  const redoOnly = { ...region, drawings: [], drawingHistory: { undo: [], redo: [step] } };
  for (const value of [undefined, { ...region, drawings: [] }, { ...undoOnly, drawingHistory: undefined }]) assert.equal(domain.hasDrawingDocument(value), false);
  for (const value of [region, undoOnly, redoOnly]) assert.equal(domain.hasDrawingDocument(value), true);
  for (const command of [{ type: 'add_drawing', drawing: vector }, { type: 'update_drawing', drawing: vector }, { type: 'remove_drawing', drawingId: vector.id }]) assert.equal(domain.usesDrawingDocument(undoOnly, command), false);
  assert.equal(domain.usesDrawingDocument(undoOnly, { type: 'undo_drawing' }), true);
  assert.equal(domain.usesDrawingDocument(redoOnly, { type: 'redo_drawing' }), true);

  // Execute the actual Canvas entry and Editor history/selection gates with no plugin.
  const canvasCode = await raw('./components/SpaceCanvas.tsx'), canvasAst = parse(canvasCode, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const canvas = canvasAst.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration;
  const begin = canvas.body.body.find(node => node.type === 'FunctionDeclaration' && node.id.name === 'beginDrawing');
  const editorCode = await raw('./components/SharedDrawingEditor.tsx'), editorAst = parse(editorCode, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const editor = editorAst.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration;
  const gates = ['editableIds', 'historyAvailable'].map(name => editor.body.body.find(node => node.type === 'VariableDeclaration' && node.declarations.some(value => value.id.name === name)));
  const mod = new SourceTextModule(transform(`import { hasDrawingDocument, richDocumentHistoryStep,makeImagePort } from 'domain';
    let session, denied=false; const props={scene:{id:'scene',background:{id:'background'}},region:undefined,documentOnly:true};props.port=makeImagePort(props);
    const blocked=()=>denied, settledRegion=(value,callback)=>callback(value), drawings=()=>props.region.drawings??[];
    const ocrRequests={cancel(){}}, cancelGesture=undefined, hideToolbar=()=>{}, setForceNew=()=>{}, setActiveItemId=()=>{}, setActive=()=>{}, setSelectionActivity=()=>{};
    const setDrawingSession=value=>{session=value};
    export ${canvasCode.slice(begin.start, begin.end)}
    ${gates.map(node => `export ${editorCode.slice(node.start, node.end)}`).join('\n')}
    export const enter=(value,block=false)=>{session=undefined;props.region=value;denied=block;beginDrawing(value);return session};`));
  await mod.link(() => synthetic({...domain,makeImagePort:imageMod.namespace.imagePort})); await mod.evaluate();
  for (const value of [undoOnly, redoOnly]) {
    assert.deepEqual(mod.namespace.enter(value), { sceneId: 'scene', regionId: region.id, backgroundId: 'background', sourceId: 'background', kind: 'core' });
    assert.equal(mod.namespace.editableIds().size, 0);
    assert.equal(mod.namespace.historyAvailable(value === redoOnly), true);
  }
  assert.equal(mod.namespace.enter(undoOnly, true), undefined);
  assert.equal(mod.namespace.enter({ ...region, drawings: [] }).kind, 'core');
  mod.namespace.enter({ ...undoOnly, drawings: [vector, drawing] }); assert.deepEqual([...mod.namespace.editableIds()], [drawing.id]);
  checks.push('Actual Canvas opens empty regions and vector-only undo/redo with core authority; independent document-only Editor still selects Rich and protects vector authoring');
}
{
  const calls = [], previewCommands = []; let native = true;
  const mod = new SourceTextModule(transform(await raw('./drawing-layout-bridge.ts')));
  await mod.link(name => name.endsWith('/core') ? synthetic({ isTauri: () => native, invoke: async (command, args) => { calls.push({ command, args }); return command === 'get_drawing_layout_preview' ? receipt(args) : undefined; } }) : name === './bridge' ? synthetic({ previewDrawingCommand: async command => { previewCommands.push(command); return { revision: 9 }; } }) : synthetic(preview)); await mod.evaluate();
  await mod.namespace.getDrawingLayoutPreview(target, 'source');
  for (const format of ['table','markdown','csv','tsv','png']) await mod.namespace.copyDrawingTable(target, format);
  assert.deepEqual(calls[0], { command: 'get_drawing_layout_preview', args: target });
  assert.deepEqual(calls.slice(1).map(value => value.args.format), ['table','markdown','csv','tsv','png']);
  assert.equal(calls.slice(1).every(value => value.command === 'copy_drawing_table'), true);
  await assert.rejects(mod.namespace.applyDrawingDocument({ type: 'add_drawing', drawing }));
  native = false; const count = calls.length;
  await assert.rejects(mod.namespace.getDrawingLayoutPreview(target, 'source'), /桌面版/); await assert.rejects(mod.namespace.copyDrawingTable(target, 'table'), /桌面版/);
  const undo = { type: 'undo_drawing', sceneId: id(1), regionId: id(2), backgroundId: asset.id, expectedRevision: 7 };
  assert.deepEqual(await mod.namespace.applyDrawingDocument(undo), { revision: 9 });
  assert.deepEqual(previewCommands, [undo]);
  assert.equal(calls.length, count);
  checks.push('Actual native bridge uses exact immutable target and five copy formats; browser history reaches the in-memory drawing model without native invocation');
}
{
  const code = await raw('./App.tsx'), ast = parse(code, { sourceType: 'module', plugins: ['typescript','jsx'] });
  const app = ast.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration;
  const functions = ['drawingDocumentCommand', 'copyDrawingTable'].map(name => app.body.body.find(node => node.type === 'FunctionDeclaration' && node.id.name === name));
  assert.equal(functions.every(Boolean), true);
  const shell = new SourceTextModule(transform(`let commands=Promise.resolve(); let exitFailure; const drawingFlushDepth=0;
    let current=${JSON.stringify({ id:id(1), background:asset, regions:[region], frozen:false, closed:false })};
    let exiting=false, recordingState=false; const scene=()=>current, exitPreparing=()=>exiting, recording=()=>recordingState, scroll=()=>false;
    import { usesDrawingDocument, drawingLayouts, spaceOperations, accept } from 'host';
    ${functions.map(node => `export ${code.slice(node.start,node.end)}`).join('\n')}
    export const setCurrent=value=>{current=value}; export const setExit=value=>{exiting=value};`));
  const calls = [], gate = deferred(); let active = 0;
  await shell.link(() => synthetic({ usesDrawingDocument: domain.usesDrawingDocument, accept: value => calls.push(['accept', value]), spaceOperations: { begin() { active++; return () => active--; } }, drawingLayouts: { applyDrawingDocument: async value => { calls.push(['document',value]); return { revision: 8 }; }, copyDrawingTable: async (...args) => { calls.push(['copy',...args]); return gate.promise; } } })); await shell.evaluate();
  await shell.namespace.drawingDocumentCommand({ sceneId:id(1), regionId:id(2), backgroundId:asset.id, expectedRevision:7, type:'update_drawing', drawing });
  assert.equal(calls[0][0], 'document'); assert.equal(calls[1][0], 'accept');
  const copy = shell.namespace.copyDrawingTable(target, 'png'); assert.equal(active, 1);
  shell.namespace.setExit(true); await assert.rejects(shell.namespace.copyDrawingTable(target, 'table'), /当前无法/); assert.equal(active,1);
  gate.resolve(); await copy; assert.equal(active,0);
  shell.namespace.setExit(false); shell.namespace.setCurrent({ id:id(100), regions:[] });
  await assert.rejects(shell.namespace.copyDrawingTable(target, 'table'), /已更新/); assert.equal(active,0);
  checks.push('Actual App handlers route document changes without a fake grant and keep copy in SpaceOperations until real settlement; exit/source changes reject new work');
}
{
  const code = await raw('./components/SharedDrawingEditor.tsx'), ast = parse(code, { sourceType:'module', plugins:['typescript','jsx'] });
  const component = ast.program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration;
  const copy = component.body.body.find(node=>node.type==='FunctionDeclaration' && node.id.name==='copySelectedTable');
  const register = component.body.body.find(node=>node.type==='VariableDeclaration' && node.declarations.some(value=>value.id.name==='unregisterFlush')).declarations[0].init;
  const callback = register.arguments[0];
  const errors = [], task = deferred(), context = { sceneId:id(1),background:asset,region };
  const mod = new SourceTextModule(transform(`import { taskCopy, richPreviewTarget, richSourceIdentity, report } from 'fixture';const props={port:{copyTable:taskCopy,sourceIdentity:()=>richSourceIdentity(context)}};
    let context=${JSON.stringify(context)}, currentDrawing=${JSON.stringify(drawing)}, disposed=false, finishing=false, tableCopy, flight, flushImages;
    const identity=()=>context.sceneId; const selectedDrawing=()=>currentDrawing, selected=()=>currentDrawing.id, richContext=()=>context;
    const disabled=()=>finishing, edit=()=>undefined, cancelGesture=undefined, tableFormat=()=>'png';
    const setFinishing=value=>{finishing=value}; const saveText=async()=>true;
    export ${code.slice(copy.start,copy.end)}
    export const flush=${code.slice(callback.start,callback.end)};
    export const changeSource=()=>{context={...context,sceneId:'other'}};
    export const setImageFlush=flush=>{flushImages=flush};
    export const isBusy=()=>finishing;`));
  let count=0;
  await mod.link(()=>synthetic({taskCopy:async()=>{count++;return task.promise;},richPreviewTarget:preview.richPreviewTarget,richSourceIdentity:preview.richSourceIdentity,report:error=>errors.push(String(error))}));await mod.evaluate();
  mod.namespace.copySelectedTable(); mod.namespace.copySelectedTable(); await tick(); assert.equal(count,1); assert.equal(mod.namespace.isBusy(),true);
  let flushed=false; const flushing=mod.namespace.flush(()=>true).then(()=>{flushed=true;}); await tick(); assert.equal(flushed,false);
  task.resolve(); await flushing; await tick(); assert.equal(mod.namespace.isBusy(),false);
  const imageTask=deferred();mod.namespace.setImageFlush(()=>imageTask.promise);
  let imageFlushed=false;const boardFlush=mod.namespace.flush(()=>true).then(()=>{imageFlushed=true;});await tick();assert.equal(imageFlushed,false);
  imageTask.resolve();await boardFlush;assert.equal(imageFlushed,true);
  mod.namespace.copySelectedTable();mod.namespace.changeSource();await tick();assert.equal(count,1);assert.equal(errors.length,0);
  checks.push('Actual Editor copy blocks duplicate clicks and its flush waits for real completion; source switch before dispatch cannot copy a successor');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
