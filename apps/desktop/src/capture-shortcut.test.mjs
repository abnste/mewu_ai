// node --experimental-vm-modules apps/desktop/src/capture-shortcut.test.mjs
// Pure editor and synthetic host promises. Never registers a system shortcut.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
const source = async file => stripTypeScriptTypes(await readFile(new URL(file, import.meta.url), 'utf8'), { mode: 'transform' });
const module = new SourceTextModule(await source('capture-shortcut-editor.ts')); await module.link(() => { throw Error('unexpected import'); }); await module.evaluate();
const { defaultCaptureShortcut, formatShortcut, shortcutFromKey, equalShortcut, acceptShortcutState, settleShortcutSave, ShortcutLeaseController } = module.namespace;
const checks = [], tick = async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const key = (value, extra = {}) => ({ key: value, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, repeat: false, isComposing: false, keyCode: 0, ...extra });
assert.deepEqual(shortcutFromKey(key('s', { shiftKey: true, altKey: true })), defaultCaptureShortcut);
assert.deepEqual(shortcutFromKey(key('a', { code: 'KeyQ', ctrlKey: true })), { code: 'KeyA', ctrl: true, shift: false, alt: false });
assert.equal(shortcutFromKey(key('Delete')), null);
for (const value of [key('a'), key('F8',{ctrlKey:true}),key('Backspace'),key('Control',{ctrlKey:true}),key('a',{ctrlKey:true,metaKey:true}),key('a',{ctrlKey:true,repeat:true}),key('a',{ctrlKey:true,isComposing:true}),key('a',{ctrlKey:true,keyCode:229})]) assert.equal(shortcutFromKey(value), undefined);
assert.equal(formatShortcut(null),'未设置'); assert.equal(formatShortcut(defaultCaptureShortcut),'Shift + Alt + S'); assert.equal(equalShortcut(null,defaultCaptureShortcut),false);
checks.push('A-Z modifier capture matches native letter keys across layouts; Delete is explicit null, repeated/IME/reserved/bare keys do not mutate');
const state = (revision=1,sequence=1,extra={}) => ({revision,sequence,configured:defaultCaptureShortcut,active:defaultCaptureShortcut,status:'active',editable:true,...extra});
{
  const current=state(2,9,{status:'cleanup_pending'});
  assert.equal(acceptShortcutState(current,state(2,8)),current); assert.equal(acceptShortcutState(current,state(1,10)),current);
  assert.equal(acceptShortcutState(current,state(2,10)).status,'active');
  const submitted={shortcut:null,expectedRevision:1};
  assert.deepEqual(settleShortcutSave(submitted,submitted,state(2,2,{configured:null,active:null}),state(2,2,{configured:null,active:null})),{conflict:false});
  const result=settleShortcutSave(submitted,submitted,state(2,2,{configured:null}),state(3,3)); assert.equal(result.conflict,true);assert.equal(result.draft.expectedRevision,2);assert.equal(result.draft.shortcut,null);
  const newer={shortcut:defaultCaptureShortcut,expectedRevision:1};assert.equal(settleShortcutSave(submitted,newer,state(2,2),state(3,3)).draft,newer);
  checks.push('Config CAS and runtime sequence are independent; successful old acknowledgement cannot upgrade draft to another writer revision');
}
function fixture() {
  const begins=[],ends=[],changes=[],records=[],errors=[];let active=true,nonce=0;
  const controller=new ShortcutLeaseController({active:()=>active,begin:id=>{const value=deferred();begins.push({...value,id});return value.promise;},end:id=>{const value=deferred();ends.push({...value,id});return value.promise;},changed:value=>changes.push(value),record:value=>records.push(value),error:error=>errors.push(String(error)),id:()=>`lease-${++nonce}`});
  const grant=i=>begins[i].resolve({leaseId:begins[i].id,expiresAt:Date.now()+30000});
  return {controller,begins,ends,changes,records,errors,grant,active:value=>active=value};
}
{
  const f=fixture(), arming=f.controller.arm();await tick();assert.equal(f.begins.length,1);assert.equal(f.controller.armed(),false);
  f.controller.record({leaseId:'lease-1',shortcut:defaultCaptureShortcut});assert.equal(f.records.length,0);
  f.grant(0);await arming;assert.equal(f.controller.armed(),true);f.controller.record({leaseId:'old',shortcut:defaultCaptureShortcut});f.controller.record({leaseId:'lease-1',shortcut:defaultCaptureShortcut});assert.equal(f.records.length,1);
  const ended=f.controller.disarm();assert.equal(f.controller.armed(),false);await tick();f.ends[0].resolve();await ended;
  checks.push('Recording begins only after granted lease; native owned chord uses matching lease and disarm is synchronous');
}
{
  const f=fixture(), first=f.controller.arm();await tick();f.active(false);const end=f.controller.disarm();f.active(true);const second=f.controller.arm();await tick();assert.equal(f.begins.length,1);
  f.grant(0);await first;await tick();assert.equal(f.ends.length,1);assert.equal(f.ends[0].id,'lease-1');assert.equal(f.begins.length,1);
  f.ends[0].resolve();await end;await tick();assert.equal(f.begins.length,2);f.grant(1);await second;assert.equal(f.controller.armed(),true);
  f.controller.ended('lease-1');f.controller.record({leaseId:'lease-1',shortcut:defaultCaptureShortcut});assert.equal(f.controller.armed(),true);assert.equal(f.records.length,0);
  const cleanup=f.controller.disarm();await tick();f.ends[1].resolve();await cleanup;
  checks.push('Blur during delayed begin ends only its own eventual lease; next focus waits begin+end before next native begin, old events cannot hijack it');
}
{
  const f=fixture(), first=f.controller.arm();const ended=f.controller.disarm();await first;await ended;assert.equal(f.begins.length,0);assert.equal(f.ends.length,0);
  const second=f.controller.arm();await tick();f.grant(0);await second;f.active(false);const stop=f.controller.disarm();const third=f.controller.arm();await third;await tick();assert.equal(f.begins.length,1);f.ends[0].resolve();await stop;
  checks.push('Canceled-before-admission focus and hidden refocus never open a host lease or replay a key');
}
{
  const f=fixture(), first=f.controller.arm();await tick();f.grant(0);await first;const cleanup=f.controller.disarm();await tick();f.ends[0].reject(Error('释放失败'));await assert.rejects(cleanup,/释放失败/);
  const second=f.controller.arm();await tick();assert.equal(f.begins.length,1);assert.equal(f.ends.length,2);assert.equal(f.ends[1].id,'lease-1');f.ends[1].resolve();await tick();assert.equal(f.begins.length,2);f.grant(1);await second;
  const last=f.controller.disarm();await tick();f.ends[2].resolve();await last;
  checks.push('Failed end is retried for that exact old ID before another begin; save flush observes release errors');
}
{
  const f=fixture(), first=f.controller.arm();await tick();f.grant(0);await first;
  const endA=f.controller.disarm();await tick();f.ends[0].reject(Error('A cleanup failed'));await assert.rejects(endA,/A cleanup/);
  const second=f.controller.arm();await tick();assert.equal(f.ends[1].id,'lease-1');
  const endB=f.controller.disarm();const third=f.controller.arm();await tick();assert.equal(f.begins.length,1);
  f.ends[1].reject(Error('A retry failed'));await assert.rejects(endB,/A retry/);await second;await tick();
  assert.equal(f.ends[2].id,'lease-1');assert.equal(f.begins.length,1);
  f.ends[2].resolve();await tick();assert.equal(f.begins.length,2);assert.equal(f.begins[1].id,'lease-3');f.grant(1);await third;
  const final=f.controller.disarm();await tick();f.ends[3].resolve();await final;
  checks.push('Blur during a failed predecessor cleanup retry carries the original lease debt through queued focus changes instead of ending an unadmitted ID');
}
{
  const f=fixture(), first=f.controller.arm();await tick();f.grant(0);await first;
  const initial=f.controller.disarm();await tick();f.ends[0].reject(Error('release failed'));await assert.rejects(initial,/release/);
  const saveA=f.controller.flush(),saveB=f.controller.flush();assert.equal(saveA,saveB);await tick();assert.equal(f.ends.length,2);assert.equal(f.ends[1].id,'lease-1');
  f.ends[1].reject(Error('retry failed'));await assert.rejects(saveA,/retry/);await assert.rejects(saveB,/retry/);await tick();assert.equal(f.ends.length,2);
  const nextSave=f.controller.flush();await tick();assert.equal(f.ends.length,3);assert.equal(f.ends[2].id,'lease-1');f.ends[2].resolve();await nextSave;
  await f.controller.flush();assert.equal(f.ends.length,3);
  checks.push('Explicit save retries the exact failed cleanup once; concurrent flush shares one retry, repeated failure waits for another explicit save');
}
{
  const f=fixture(), first=f.controller.arm();await tick();f.begins[0].resolve({leaseId:'lease-1',expiresAt:1});await first;assert.equal(f.controller.armed(),true);f.controller.ended('lease-1');assert.equal(f.controller.armed(),false);await tick();f.ends[0].resolve();await tick();
  const second=f.controller.arm();await tick();f.controller.dispose();f.grant(1);await second;await tick();f.ends[1].resolve();await tick();assert.equal(f.controller.armed(),false);assert.equal(f.records.length,0);
  checks.push('Wall-clock expiry is auxiliary; exact host-ended event is authoritative, disposal releases delayed grant without publishing new state');
}
const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const [key,value]of Object.entries(values))this.setExport(key,value);});
async function bridgeFixture(native,failAt='') {
  const calls=[],listeners=new Map(),stopped=[];const mod=new SourceTextModule(await source('shortcut-bridge.ts'));
  await mod.link(name=>name.endsWith('/core')?synthetic({isTauri:()=>native,invoke:async(name,args)=>{calls.push({name,args});return{};}}):synthetic({listen:async(name,callback,options)=>{if(name===failAt)throw Error('listen failed');listeners.set(name,{callback,options});return()=>stopped.push(name);}}));await mod.evaluate();return{api:mod.namespace,calls,listeners,stopped};
}
{
  const f=await bridgeFixture(true), states=[],recorded=[],ended=[];const stop=await f.api.subscribeCaptureShortcut({state:value=>states.push(value),recorded:value=>recorded.push(value),ended:value=>ended.push(value)});
  for(const value of f.listeners.values())assert.deepEqual(value.options,{target:'settings'});
  f.listeners.get('capture-shortcut-state').callback({payload:state()});f.listeners.get('capture-shortcut-recorded').callback({payload:{leaseId:'a',shortcut:defaultCaptureShortcut}});f.listeners.get('capture-shortcut-edit-ended').callback({payload:{leaseId:'a'}});
  await f.api.getCaptureShortcut();await f.api.beginCaptureShortcutEdit('a');await f.api.endCaptureShortcutEdit('a');await f.api.setCaptureShortcut(1,null);stop();
  assert.deepEqual(f.calls,[{name:'get_capture_shortcut',args:undefined},{name:'begin_capture_shortcut_edit',args:{leaseId:'a'}},{name:'end_capture_shortcut_edit',args:{leaseId:'a'}},{name:'set_capture_shortcut',args:{expectedRevision:1,shortcut:null}}]);assert.equal(states.length,1);assert.equal(recorded.length,1);assert.deepEqual(ended,['a']);assert.equal(f.stopped.length,3);
  const broken=await bridgeFixture(true,'capture-shortcut-edit-ended');await assert.rejects(broken.api.subscribeCaptureShortcut({state(){},recorded(){},ended(){}}));assert.equal(broken.stopped.length,2);
  const browser=await bridgeFixture(false);assert.equal((await browser.api.getCaptureShortcut()).editable,false);await assert.rejects(browser.api.setCaptureShortcut(0,null),/桌面版/);await assert.rejects(browser.api.beginCaptureShortcutEdit('a'),/桌面版/);assert.equal(browser.calls.length,0);
  checks.push('Four exact host commands, settings-scoped three event listeners with partial cleanup, browser no simulated registration');
}
{
  const directory = new URL('../src-tauri/capabilities/', import.meta.url);
  const capabilities = await Promise.all((await readdir(directory)).filter(name => name.endsWith('.json')).map(async name => JSON.parse(await readFile(new URL(name, directory), 'utf8'))));
  const conf = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  const settings = capabilities.find(value => value.identifier === 'settings'), space = capabilities.find(value => value.identifier === 'space');
  assert.deepEqual(settings.windows, ['settings']); assert.ok(conf.app.security.capabilities.includes('settings'));
  for (const permission of ['allow-set-capture-shortcut','allow-begin-capture-shortcut-edit','allow-end-capture-shortcut-edit']) {
    assert.ok(settings.permissions.includes(permission));
    assert.deepEqual(capabilities.filter(value => value.permissions.includes(permission)).map(value => value.identifier), ['settings']);
  }
  assert.ok(settings.permissions.includes('allow-get-capture-shortcut')); assert.ok(space.permissions.includes('allow-get-capture-shortcut'));
  checks.push('Real native capability JSON grants edit only to settings; space has read only and the settings capability is active');
}
{
  // Compile the actual TSX: Solid delegates onKeyDown to document, where the
  // already-mounted dialog listener wins. The recorder needs a native target
  // handler so Escape is consumed before that real parent listener.
  const text=await readFile(new URL('components/CaptureShortcutField.tsx',import.meta.url),'utf8');
  const compiled=transformSync(text,{filename:'CaptureShortcutField.tsx',parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'dom'}]],configFile:false,babelrc:false,ast:true});
  const nativeListenerNames=compiled.ast.program.body.filter(node=>node.type==='ImportDeclaration'&&node.source.value==='solid-js/web').flatMap(node=>node.specifiers.filter(value=>value.imported?.name==='addEventListener').map(value=>value.local.name));
  let nativeKeyboard=false;
  const visit=node=>{if(!node||typeof node!=='object')return;if(node.type==='CallExpression'&&node.callee?.type==='Identifier'&&nativeListenerNames.includes(node.callee.name)&&node.arguments?.[1]?.value==='keydown'&&node.arguments?.[2]?.name==='keyboard'&&node.arguments?.[3]?.value!==true)nativeKeyboard=true;for(const value of Object.values(node)){if(Array.isArray(value))value.forEach(visit);else if(value&&typeof value==='object')visit(value);}};
  visit(compiled.ast);assert.equal(nativeKeyboard,true);
  checks.push('Actual Solid compiled recorder installs a native input keydown listener; Escape cannot run only after the dialog document listener');
}
console.log(JSON.stringify({passed:checks.length,checks},null,2));
