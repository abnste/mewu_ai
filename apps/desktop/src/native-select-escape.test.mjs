// Actual production Escape callbacks; synthetic DOM/IPC only, no native devices.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import vm from 'node:vm';
const source = file => readFile(new URL(file, import.meta.url), 'utf8');
const ts = value => stripTypeScriptTypes(value, { mode: 'transform' });
async function callback(file, name) {
  const code = await source(file), ast = parse(code, { sourceType:'module', plugins:['typescript','jsx'] });
  let found;
  function visit(node) {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'FunctionDeclaration' && node.id?.name === name) found = node;
    if (node.type === 'VariableDeclarator' && node.id?.name === name) found = node.init;
    for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === 'object') visit(value);
  }
  visit(ast); assert.ok(found, `${file} ${name}`);
  return found.type === 'FunctionDeclaration' ? `(${ts(code.slice(found.start,found.end))})` : ts(code.slice(found.start,found.end));
}
const helperSource = await source('./native-select-escape.ts');
const helperNode = parse(helperSource,{sourceType:'module',plugins:['typescript']}).program.body.find(node=>node.type==='ExportNamedDeclaration').declaration;
const helper = ts(helperSource.slice(helperNode.start,helperNode.end));
class Element {
  constructor(tagName = 'div', parentElement = null) { this.tagName = tagName; this.parentElement = parentElement; }
  closest(selector) {
    const tags = selector.split(',');
    for (let current = this; current; current = current.parentElement) if (tags.includes(current.tagName)) return current;
    return null;
  }
}
function fixture(code) {
  const effects = [], root = new Element(), select = new Element('select'), option = new Element('option',select);
  const doc = { activeElement:root, querySelector:selector => selector === 'select:open' ? doc.picker : null, picker:null };
  const effect = (...value) => { effects.push(value); };
  const context = vm.createContext({
    Element, document:doc, effects, console,
    props:{onClose:()=>effect('close'),inputLocked:false,scene:{items:[]}},
    cancelGesture:undefined, disabled:()=>false, edit:()=>undefined,
    exitPreparing:()=>false,settings:()=>false,pendingDrawingDrafts:()=>[],scroll:()=>undefined,recording:()=>undefined,
    busy:()=>false,sessionsOpen:()=>false,scene:()=>({id:'scene'}),expanded:()=>({}),closeScene:()=>effect('closeScene'),
    voicePending:()=>false, pendingPin:true,cancelPendingPin:()=>effect('cancelPin'),
    cancelPinOnEscape:(event,pending,cancel)=>{if(event.key==='Escape'&&pending){cancel();event.preventDefault();event.stopImmediatePropagation();}},
    videoExport:()=>true,cancelVideoExport:()=>effect('cancelExport'), activeItemId:()=>'',
    menu:()=>true,setMenu:()=>effect('closeMenu'),confirmRemoval:()=>true,setConfirmRemoval:()=>effect('closeRemoval'),
    closing:false,gesture:{end:()=>effect('end')},pin:{controlPin:value=>{effect(value.type);return Promise.resolve();}},report:()=>{},
    status:()=>({phase:'recording'}),stopOrCancel:()=>effect('stopRecording'),
    act:value=>effect(value),stopAnnotationPointer:()=>effect('stopPointer'),manualView:()=>({move:{}}),manual:{cancelMove:()=>effect('cancelMove')},
    cancelPointer:()=>effect('cancelPointer'),
  });
  // Compile the real shared guard in the same DOM realm as each real callback.
  vm.runInContext(helper+`\nglobalThis.run = ${code};`,context);
  const event = target => ({ key:'Escape',keyCode:27,type:'keydown',target,isComposing:false,repeat:false,preventDefault:()=>effect('prevent'),stopPropagation:()=>effect('stop'),stopImmediatePropagation:()=>effect('stopImmediate') });
  return {context,root,select,option,doc,effects,event,run:context.run};
}
const callbacks = [
  ['Shared drawing editor','./components/SharedDrawingEditor.tsx','keyboard'],
  ['Space close','./App.tsx','escape'],
  ['Pending pin/voice','./App.tsx','pinEscape'],
  ['Video export','./App.tsx','videoExportEscape'],
  ['Space selection','./components/SpaceCanvas.tsx','recordKey'],
  ['Settings dialog','./components/SettingsDialog.tsx','keyboard'],
  ['Pinned image','./PinView.tsx','keyboard'],
  ['Connections panel','./components/ConnectionsPanel.tsx','keyboard'],
  ['Plugins panel','./components/PluginsPanel.tsx','keyboard'],
  ['Recorder','./RecordingSurface.tsx','keyboard'],
  ['Scroll capture','./ScrollSurface.tsx','keyboard'],
  ['Unsaved drawing dialog','./components/DrawingDraftsDialog.tsx','keydown'],
  ['Video trim','./components/VideoTrimBar.tsx','keyDown'],
  ['Video annotation move','./components/VideoArtifact.tsx','escapeMove'],
];
for (const [label,file,name] of callbacks) test(`${label}: native picker Escape never closes/cancels content; normal Escape retains behavior`, async () => {
  const f = fixture(await callback(file,name));
  if (name==='recordKey') f.context.cancelGesture=()=>f.effects.push(['cancelGesture']);
  for (const target of [f.select,f.option,new Element('span',f.option)]) { f.run(f.event(target)); assert.equal(f.effects.length,0); }
  f.doc.activeElement=f.select; f.run(f.event(f.doc)); f.run(f.event(f.root)); assert.equal(f.effects.length,0);
  f.doc.activeElement=f.root; f.doc.picker=f.select; f.run(f.event(f.root)); assert.equal(f.effects.length,0);
  f.doc.picker=null; f.run(f.event(f.root)); await Promise.resolve();
  assert.ok(f.effects.length>0,'ordinary Escape must still perform the existing close/cancel');
});
test('guard preserves legacy select escape, handles absent targets and leaves other keys to existing handlers',()=>{
  const f=fixture('(event)=>nativeSelectOwnsEscape(event)');
  f.doc.querySelector=()=>{throw new SyntaxError('unsupported :open');};
  assert.equal(f.run(f.event(f.select)),true);assert.equal(f.run(f.event(f.option)),true);
  assert.equal(f.run(f.event(f.root)),false);f.doc.activeElement=f.select;assert.equal(f.run(f.event(null)),true);
  assert.equal(f.run({...f.event(f.select),key:'Tab'}),false);assert.equal(f.run({...f.event(f.select),key:'Enter'}),false);
  assert.equal(f.effects.length,0,'guard must never consume native default or propagation');
});
