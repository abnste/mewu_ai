// Synthetic source identity, async lifecycle, placement and exact IPC. No desktop/clipboard.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
const source = async name => stripTypeScriptTypes(await readFile(new URL(name, import.meta.url), 'utf8'), {mode:'transform'});
const load = async name => { const m=new SourceTextModule(await source(name));await m.link(()=>{throw Error('unexpected import')});await m.evaluate();return m.namespace; };
const {codeSource,CodeScanController,validCodeResult}=await load('code-scan.ts');
const {placeCodeCard}=await load('code-card-layout.ts');
const checks=[];
const bg=()=>({id:'bg',name:'source',kind:'image',path:'owned',width:1200,height:800});
const scene=()=>({id:'scene',closed:false,frozen:false,background:bg(),regions:[{id:'r',x:10,y:20,width:500,height:300,drawingRevision:0,drawings:[]}],messages:[],draft:'',refs:[]});
const plugin=(id='mewu.codes',official=true)=>({manifest:{id,contributions:[{id:'recognize',kind:'selection.codes',engine:'rxing'}]},source:{type:official?'official':'local'},state:'enabled',revision:1});
const input=()=>codeSource(scene(),'r',[plugin()]);
{
 const original=scene(),key=codeSource(original,'r',[plugin()]).key;
 for (const mutate of [s=>s.draft='new',s=>s.messages.push({text:'stream'}),s=>s.refs.push({id:'r'}),s=>s.regions[0].ocr={document:{}},s=>s.regions[0].drawingHistory={undo:[{}]},s=>s.run={id:'run',status:'running'}]) {const s=structuredClone(original);mutate(s);assert.equal(codeSource(s,'r',[plugin()]).key,key);}
 for(const mutate of [s=>s.background.path='same-id-new-path',s=>s.background.scaleFactor=2,s=>s.regions[0].x++,s=>s.regions[0].drawingRevision++,s=>s.regions[0].imageOverride={...bg(),id:'long'},s=>s.regions[0].drawings=[{id:'d',kind:'rect',color:'#112233',strokeWidth:2,points:[{x:1,y:1},{x:20,y:20}]}],s=>s.regions[0].translation={backgroundId:'bg',sourceId:'bg',drawingRevision:0,document:{width:500,height:300},overlay:{...bg(),id:'overlay'}}]) {const s=structuredClone(original);mutate(s);assert.notEqual(codeSource(s,'r',[plugin()]).key,key);}
 const translated=scene();translated.regions[0].translation={backgroundId:'bg',sourceId:'bg',drawingRevision:0,document:{width:500,height:300},overlay:{...bg(),id:'overlay'}};const t=codeSource(translated,'r',[plugin()]).key;translated.regions[0].translation.overlay.path='changed';assert.notEqual(codeSource(translated,'r',[plugin()]).key,t);
 assert.equal(codeSource(original,'r',[plugin('z'),plugin(),plugin('a')]).request.pluginId,'mewu.codes');assert.equal(codeSource(original,'r',[plugin('z',false),plugin('a',false)]).request.pluginId,'a');assert.equal(codeSource(original,'r',[{...plugin(),state:'disabled'}]),undefined);
 checks.push('Complete persisted pixel identity, deterministic grant, ignores draft/AI/OCR/history');
}
const defer=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no});return{promise,resolve,reject}};
const tick=async()=>{for(let i=0;i<14;i++)await Promise.resolve();await new Promise(r=>setTimeout(r,3));for(let i=0;i<14;i++)await Promise.resolve()};
function fixture(){let n=0,view;const runs=[],stops=[];const c=new CodeScanController({delay:0,id:()=>`id-${++n}`,matches:()=>true,run:r=>{const d=defer();runs.push({...d,r});return d.promise},cancel:id=>{const d=defer();stops.push({...d,id});return d.promise},changed:v=>view=v});const result=i=>({requestId:runs[i].r.requestId,sceneId:'scene',regionId:'r',sourceToken:`token-${i}`,codes:[{id:'1',format:'QR_CODE',text:'https://example.invalid',canOpen:true}]});return{c,runs,stops,result,view:()=>view}}
{
 const f=fixture(),a=input();f.c.setSource(a);await tick();assert.equal(f.runs.length,1);f.c.setSource({...a},true);f.runs[0].resolve(f.result(0));await tick();assert.ok(f.view());assert.equal(f.c.isCurrent('token-0'),false);f.c.setSource(a,false);assert.equal(f.c.isCurrent('token-0'),true);await tick();assert.equal(f.runs.length,1);
 f.c.setSource(undefined);await tick();assert.equal(f.view(),undefined);assert.equal(f.stops.length,1);f.c.setSource(a);await tick();assert.equal(f.runs.length,1);f.stops[0].resolve();await tick();assert.equal(f.runs.length,2);f.runs[1].resolve(f.result(1));await tick();assert.equal(f.c.isCurrent('token-1'),true);f.c.dispose();await tick();f.stops[1].resolve();await tick();
 checks.push('Paused UI preserves completed authority; leaving and revisiting source obtains fresh request/token');
}
{
 const f=fixture(),a=input(),b={...input(),key:'B'},c={...input(),key:'C'};f.c.setSource(a);await tick();f.c.setSource(b);f.c.setSource(c);await tick();assert.equal(f.stops.length,1);f.stops[0].resolve();await tick();assert.equal(f.runs.length,1);f.runs[0].resolve(f.result(0));await tick();assert.equal(f.view(),undefined);assert.equal(f.runs.length,2);assert.equal(f.runs[1].r.requestId,'id-2');f.runs[1].resolve(f.result(1));await tick();assert.equal(f.view().key,'C');f.c.dispose();await tick();f.stops[1].resolve();await tick();
 checks.push('One actual in-flight across cancel and keyed Canvas replacement; latest intent only, late result discarded');
}
{
 const f=fixture(),a=input();f.c.setSource(a);await tick();f.runs[0].resolve(f.result(0));await tick();f.c.invalidate('old');assert.ok(f.view());f.c.invalidate('id-1');await tick();f.stops[0].resolve();await tick();assert.equal(f.view(),undefined);f.c.setSource(a);await tick();assert.equal(f.runs.length,1);f.c.setSource(undefined);f.c.setSource(a);await tick();assert.equal(f.runs.length,2);f.runs[1].reject(Error('decoder'));await tick();f.c.setSource({...a});await tick();assert.equal(f.runs.length,2);f.c.dispose();await tick();f.stops[1].resolve();await tick();
 checks.push('Exact invalidation cannot revoke a newer request; real focus generation restores, automatic errors do not retry-loop');
}
{
 const f=fixture();f.c.setSource(input());await tick();const exiting=f.c.cancel();let done=false;void exiting.then(()=>done=true);await tick();f.runs[0].resolve(f.result(0));await tick();assert.equal(done,false);f.stops[0].resolve();await exiting;assert.equal(done,true);assert.equal(f.view(),undefined);f.c.dispose();
 checks.push('Exit cancellation waits both actual scan and cancellation and never publishes late result');
}
{
 const req={...input().request,requestId:'x'},result={requestId:'x',sceneId:'scene',regionId:'r',sourceToken:'token',codes:[{id:'1',format:'QR_CODE',text:'<script>literal</script>',canOpen:false}]};assert.ok(validCodeResult(result,req));assert.ok(!validCodeResult({...result,requestId:'old'},req));assert.ok(!validCodeResult({...result,codes:[...result.codes,...result.codes]},req));assert.ok(!validCodeResult({...result,codes:[{...result.codes[0],text:'😀'.repeat(2049)}]},req));assert.ok(!validCodeResult({...result,codes:Array.from({length:12},(_,i)=>({...result.codes[0],id:String(i),text:'x'.repeat(8192)}))},req));checks.push('Bounded Unicode output and exact request/region identity; payload remains text');
}
{
 const region={x:120,y:160,width:500,height:300},toolbar={x:120,y:110,width:500,height:46},size={width:280,height:40};const p=placeCodeCard({width:1024,height:700},region,size,[toolbar,{x:200,y:540,width:600,height:140}]);assert.deepEqual(p,{x:120,y:62,...size});const narrow=placeCodeCard({width:330,height:500},{x:0,y:0,width:330,height:400},size,[{x:0,y:410,width:330,height:90}]);assert.ok(narrow&&narrow.x>=6&&narrow.x+narrow.width<=324);assert.equal(placeCodeCard({width:100,height:100},region,size,[]),undefined);assert.equal(placeCodeCard({width:500,height:500},region,size,[{x:0,y:0,width:500,height:500}]),undefined);checks.push('Compact placement avoids toolbar/composer, clips to narrow viewport and hides when no safe room');
}
{
 const calls=[],events=[];let listener;const m=new SourceTextModule(await source('code-bridge.ts'));
 const core=new SyntheticModule(['invoke','isTauri'],function(){this.setExport('isTauri',()=>true);this.setExport('invoke',async(n,args)=>{calls.push([n,args]);});});const event=new SyntheticModule(['listen'],function(){this.setExport('listen',async(n,callback,options)=>{events.push([n,options]);listener=callback;return()=>events.push(['off'])});});
 await m.link(n=>n.endsWith('/core')?core:event);await m.evaluate();const b=m.namespace;let id;const stop=await b.subscribeCodeInvalidated(v=>id=v);listener({payload:{requestId:'exact'}});assert.equal(id,'exact');const request={...input().request,requestId:'req'};await b.scanCodes(request);await b.cancelCodeScan('req');await b.actOnCode('token','code','open');stop();assert.deepEqual(calls,[['scan_plugin_codes',request],['cancel_plugin_code_scan',{requestId:'req'}],['act_on_code',{sourceToken:'token',codeId:'code',action:'open'}]]);assert.deepEqual(events,[['code-scan-invalidated',{target:'space'}],['off']]);
 const m2=new SourceTextModule(await source('code-bridge.ts'));const unsupported=new SyntheticModule(['invoke','isTauri'],function(){this.setExport('isTauri',()=>false);this.setExport('invoke',()=>{throw Error('must not invoke')})});await m2.link(n=>n.endsWith('/core')?unsupported:event);await m2.evaluate();await assert.rejects(m2.namespace.scanCodes(request),/桌面版/);await assert.rejects(m2.namespace.actOnCode('t','c','copy'),/桌面版/);checks.push('Native bridge passes only typed IDs/actions; subscription precedes work; browser honestly unsupported');
}
{
 const base=new URL('../src-tauri/',import.meta.url),conf=JSON.parse(await readFile(new URL('tauri.conf.json',base),'utf8'));
 const names=['allow-scan-plugin-codes','allow-cancel-plugin-code-scan','allow-act-on-code'];
 for(const filename of await readdir(new URL('capabilities/',base))){if(!filename.endsWith('.json'))continue;const value=JSON.parse(await readFile(new URL('capabilities/'+filename,base),'utf8'));const granted=names.filter(name=>value.permissions.includes(name));if(value.identifier==='space'){assert.ok(conf.app.security.capabilities.includes(value.identifier));assert.deepEqual(value.windows,['space']);assert.deepEqual(granted,names);}else assert.equal(granted.length,0);}
 checks.push('Only active first-party space capability receives all three code IPC permissions');
}
console.log(JSON.stringify({passed:checks.length,checks},null,2));
