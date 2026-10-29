// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule,SyntheticModule} from 'node:vm';
const load=async(path)=>new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'}));
const contract=JSON.parse(await readFile(new URL('../../../packages/sdk/appearance.json',import.meta.url),'utf8'));
const values=new SyntheticModule(['default'],function(){this.setExport('default',contract);});
const policy=await load('./appearance.ts');await policy.link(()=>values);await policy.evaluate();
const controller=await load('./appearance-controller.ts');await controller.link(()=>policy);await controller.evaluate();
const {parseAppearanceTheme,appearanceProperties,mergeAppearanceRecords,appearanceChoices,readAppearanceSelection}=policy.namespace;
const {AppearanceController}=controller.namespace;
const plain=v=>JSON.parse(JSON.stringify(v));
const png='iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==';
const record=(pluginId,revision=1,themes=[{serviceId:'theme',title:pluginId,theme:{version:1,base:'dark',colors:{accent:'#AADDEE'},backgrounds:{settings:{png,fit:'cover'}}}}])=>({pluginId,revision,themes});
test('appearance accepts closed semantic tokens and bounded inert PNGs',()=>{
  assert.ok(parseAppearanceTheme(record('alpha').themes[0].theme));
  for(const theme of [{version:2,base:'dark'},{version:1,base:'dark',css:'body{}'},{version:1,base:'dark',colors:{position:'#aabbcc'}},{version:1,base:'dark',colors:{accent:'url(https://x)'}},{version:1,base:'dark',backgrounds:{settings:{png:'data:image/svg+xml;base64,AAAA',fit:'cover'}}}])assert.equal(parseAppearanceTheme(theme),null);
  const bytes=Buffer.from(png,'base64');bytes.writeUInt32BE(10000,16);
  assert.equal(parseAppearanceTheme({version:1,base:'light',backgrounds:{settings:{png:bytes.toString('base64'),fit:'cover'}}}),null);
  const props=appearanceProperties(parseAppearanceTheme(record('alpha').themes[0].theme));
  assert.equal(props['--accent'],'#AADDEE');assert.equal(props['--text'],contract.colors.foreground.dark);
  assert.match(props['--theme-settings-image'],/^url\("data:image\/png;base64,/);
  assert.deepEqual(plain(appearanceProperties(null)),{});
});
test('two independent namespaces survive replacement, stop, removal and reinstallation',()=>{
  let records=mergeAppearanceRecords([],[record('alpha'),record('beta')]);assert.equal(appearanceChoices(records).length,2);
  records=mergeAppearanceRecords(records,[record('alpha',2,[])]);assert.equal(appearanceChoices(records).length,1);
  records=mergeAppearanceRecords(records,[record('alpha',1)]);assert.equal(appearanceChoices(records).length,1);
  records=mergeAppearanceRecords(records,[record('alpha',3)]);assert.equal(appearanceChoices(records).length,2);
  records=mergeAppearanceRecords(records,[record('alpha',4,[])]);assert.equal(appearanceChoices(records).length,1);
  records=mergeAppearanceRecords(records,[record('alpha',5)]);assert.equal(appearanceChoices(records).length,2);
});
test('selection is explicit and survives a temporary disabled contribution',async()=>{
  let notify,state,applied;const c=new AppearanceController({get:async()=>[record('alpha'),record('beta')],subscribe:async fn=>{notify=fn;return()=>{};},apply:v=>applied=v,publish:v=>state=v});
  await c.initialize();assert.equal(applied,null);
  c.select({pluginId:'alpha',serviceId:'theme'});assert.equal(applied.pluginId,'alpha');
  notify([record('alpha',2,[])]);assert.equal(applied,null);assert.equal(state.selection.pluginId,'alpha');
  notify([record('alpha',1)]);assert.equal(applied,null);
  notify([record('alpha',3)]);assert.equal(applied.pluginId,'alpha');
  c.select({pluginId:'beta',serviceId:'theme'});assert.equal(applied.pluginId,'beta');
  c.select(null);assert.equal(applied,null);c.dispose();
});
test('initial read cannot overwrite a newer revocation and late subscribe cleans up',async()=>{
  let releaseRead,notify,state,applied,stops=0;
  const c=new AppearanceController({get:()=>new Promise(resolve=>releaseRead=resolve),subscribe:async fn=>{notify=fn;return()=>stops++;},apply:v=>applied=v,publish:v=>state=v});
  c.select({pluginId:'alpha',serviceId:'theme'});const pending=c.initialize();await new Promise(setImmediate);
  notify([record('alpha',2,[])]);releaseRead([record('alpha')]);await pending;assert.equal(applied,null);assert.equal(state.ready,true);c.dispose();assert.equal(stops,1);
  let releaseStop;const second=new AppearanceController({get:async()=>{throw Error('unexpected get');},subscribe:()=>new Promise(resolve=>releaseStop=resolve),apply:()=>{},publish:()=>{}});
  const initialization=second.initialize();second.dispose();releaseStop(()=>stops++);await initialization;assert.equal(stops,2);
});
test('selection contains only structured plugin/service identities',()=>{
  assert.deepEqual(plain(readAppearanceSelection('{"pluginId":"a.b","serviceId":"theme"}')),{pluginId:'a.b',serviceId:'theme'});
  for(const raw of ['{}','[]','null','{"pluginId":"x","serviceId":"y","css":"*{}"}','{"pluginId":"../x","serviceId":"y"}'])assert.equal(readAppearanceSelection(raw),null);
});
