import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import { parse } from '@babel/parser';
import { runInNewContext } from 'node:vm';
import * as solid from 'solid-js';
import * as web from 'solid-js/web';

const raw = path => readFile(new URL(path, import.meta.url), 'utf8');
const ts = text => stripTypeScriptTypes(text, { mode: 'transform' });
const api = await import(`data:text/javascript;base64,${Buffer.from(ts(await raw('run-journal.ts'))).toString('base64')}`);
const { JournalReader, ContinuationController, ProjectionPreview, latestReply } = api;
const exitApi = await import(`data:text/javascript;base64,${Buffer.from(ts(await raw('exit-preparation.ts'))).toString('base64')}`);
const checks = [];
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve,reject; const promise=new Promise((yes,no)=>{resolve=yes;reject=no;}); return {promise,resolve,reject}; };
const identity = { sceneId:'scene-a',runId:'run-a' };
const summary = extra => ({runId:'run-a',sceneId:'scene-a',agentId:'agent-a',userMessageId:'message-a',kind:'chat',status:'interrupted',revision:7,checkpointSeq:3,createdAt:1,updatedAt:2,canContinue:true,continueBlockedReason:null,...extra});
const entry = (id='event-1',extra={}) => ({id,sequence:1,kind:'tool',phase:'responded',round:0,toolCallId:'call-1',label:'合成工具',startedAt:1,finishedAt:2,returnedError:false,contentBytes:20,contentSha256:'hash',contentUnavailable:null,selectable:true,...extra});
const decision = extra => ({sourceRunId:'run-a',journalRevision:7,checkpointSeq:3,budgetBytes:262144,projectionBytes:80,defaultProjectionBytes:80,defaultEventIds:['event-1'],selectedEventIds:['event-1'],selectionRequired:false,ready:true,blockedReason:null,...extra});
const page = extra => ({summary:summary(),entries:[entry()],continuation:decision(),nextCursor:null,...extra});
const scope = () => ({sceneId:'scene-a',agentId:'agent-a',viewedRunId:'run-a',sourceRunId:'run-a',currentRunId:'run-a',connectionId:'connection-a',connectionRevision:2,journalRevision:7,checkpointSeq:3});

{
  const calls=[],requests=[],updates=[];
  const reader=new JournalReader({page:input=>{calls.push(input);const task=deferred();requests.push(task);return task.promise;},detail:async()=>{throw Error('unused');}},state=>updates.push(state));
  reader.target(identity); assert.equal(calls.length,0); const opening=reader.open();reader.open();assert.equal(calls.length,1);
  requests[0].resolve(page());await opening;assert.equal(reader.value().page.entries.length,1);
  reader.close();await reader.open();assert.equal(calls.length,1);
  reader.target({...identity,runId:'run-b'});const second=reader.open();requests[1].resolve(null);await second;assert.equal(reader.value().legacy,true);assert.equal(reader.value().page,undefined);
  checks.push('Reads are explicit and cached only within the run; legacy null stays empty instead of fabricating tool evidence');
}
{
  const tasks=[],calls=[];const reader=new JournalReader({page:input=>{calls.push(input);const task=deferred();tasks.push(task);return task.promise;},detail:async()=>{throw Error('unused');}},()=>{});
  reader.target(identity);const a=reader.open();reader.target({...identity,runId:'run-b'});reader.open();reader.target({...identity,runId:'run-c'});reader.open();assert.equal(calls.length,1);
  tasks[0].resolve(page());await a;await tick();assert.equal(calls.length,2);assert.equal(calls[1].runId,'run-c');assert.equal(reader.value().page,undefined);
  tasks[1].resolve(page({summary:summary({runId:'run-c'})}));await tick();assert.equal(reader.value().page.summary.runId,'run-c');reader.dispose();
  checks.push('Late scene/run pages cannot publish; one real page flight keeps only the latest requested run');
}
{
  let n=0;const detailTask=deferred();const reader=new JournalReader({page:async()=>++n===1?page({nextCursor:'next'}):page({summary:summary({revision:8}),entries:[entry('event-2',{sequence:2})]}),detail:()=>detailTask.promise},()=>{});
  reader.target(identity);await reader.open();const detail=reader.showDetail('event-1');await reader.more();assert.equal(reader.value().page.entries.length,1);assert.match(reader.value().error,/更新/);
  reader.target({...identity,runId:'run-b'});detailTask.resolve({runId:'run-a',journalRevision:7,entry:entry(),literal:'OLD',unavailable:null});await detail;assert.equal(reader.value().detail,undefined);
  checks.push('Pagination cannot mix journal revisions, and old plaintext details cannot leak into another run');
}
function continuing() {
  let current=scope(), allowed=true,draft='draft A',refs=['region-a'],operations=0;
  const flushTask=deferred(),runTask=deferred(),calls=[],errors=[],accepted=[],pending=[];
  const controller=new ContinuationController({current:()=>current,allowed:()=>allowed,beginOperation:()=>{operations++;return()=>operations--;},flush:()=>flushTask.promise,enqueue:fn=>fn(),start:input=>{calls.push(input);return runTask.promise;},accept:value=>accepted.push(value),pending:value=>pending.push(value),error:value=>errors.push(value)});
  return {controller,flushTask,runTask,calls,errors,accepted,pending,operations:()=>operations,current:value=>current=value,allowed:value=>allowed=value,type:value=>draft=value,refs:value=>refs=value,values:()=>({draft,refs})};
}
{
  const f=continuing();const one=f.controller.continue(summary(),decision()),two=f.controller.continue(summary(),decision());assert.equal(one,two);assert.equal(f.operations(),1);
  f.type('draft B');f.refs(['region-a','file-b']);f.flushTask.resolve();await tick();assert.equal(f.calls.length,1);assert.equal(f.calls[0].selectedEventIds,null);assert.equal(Object.hasOwn(f.calls[0],'draft'),false);assert.equal(Object.hasOwn(f.calls[0],'refs'),false);
  f.type('draft C');f.runTask.resolve({revision:9});assert.equal(await one,true);assert.deepEqual(f.values(),{draft:'draft C',refs:['region-a','file-b']});assert.equal(f.operations(),0);
  checks.push('Default continuation is a single explicit ID-only request; flush and accepted start never consume current or newly typed draft/refs');
}
{
  for (const mutate of [f=>f.current({...scope(),sceneId:'scene-b'}),f=>f.current({...scope(),connectionRevision:3}),f=>f.current({...scope(),currentRunId:'new-run'}),f=>f.allowed(false),f=>f.controller.dispose()]) {
    const f=continuing(),promise=f.controller.continue(summary(),decision());mutate(f);f.flushTask.resolve();assert.equal(await promise,false);assert.equal(f.calls.length,0);assert.equal(f.operations(),0);
  }
  const f=continuing(),promise=f.controller.continue(summary(),decision());f.flushTask.reject(Error('未保存标注'));assert.equal(await promise,false);assert.equal(f.calls.length,0);assert.equal(f.values().draft,'draft A');assert.deepEqual(f.errors,['未保存标注']);
  checks.push('Scene/run/connection changes, exit, unmount and failed prerequisite flush all prevent model admission without clearing drafts');
}
{
  const f=continuing();assert.equal(await f.controller.continue(summary({canContinue:false,continueBlockedReason:'no_confirmed_content'}),decision()),false);
  assert.equal(await f.controller.continue(summary(),decision({selectionRequired:true,ready:false})),false);
  assert.equal(await f.controller.continue(summary(),decision({selectionRequired:true,defaultEventIds:['event-1','event-2'],selectedEventIds:['event-1']}),['event-2']),false);
  const promise=f.controller.continue(summary(),decision({selectionRequired:true,defaultEventIds:['event-1','event-2'],selectedEventIds:['event-1']}),['event-1']);f.flushTask.resolve();await tick();assert.deepEqual(f.calls[0].selectedEventIds,['event-1']);f.runTask.reject(Error('journal conflict'));assert.equal(await promise,false);assert.equal(f.calls.length,1);
  checks.push('No evidence yields no continuation; only host-validated over-budget subsets are sent, and CAS rejection never auto-retries');
}
{
  const f=continuing();f.current({...scope(),viewedRunId:'continuation-b',currentRunId:'continuation-b'});
  const promise=f.controller.continue(summary({runId:'continuation-b',kind:'continuation',revision:20,checkpointSeq:4}),decision());
  f.flushTask.resolve();await tick();assert.equal(f.calls[0].sourceRunId,'run-a');assert.equal(f.calls[0].expectedJournalRevision,7);assert.equal(f.calls[0].expectedCheckpointSeq,3);f.runTask.resolve({revision:21});await promise;
  checks.push('Retrying an interrupted continuation uses the canonical original source and revision supplied by host decision, never the short continuation action as a new question');
}
{
  const tasks=[],calls=[],values=[];const preview=new ProjectionPreview({preview:input=>{calls.push(input);const task=deferred();tasks.push(task);return task.promise;},changed:(value,error)=>values.push({value,error})});
  const input={sceneId:'scene-a',sourceRunId:'run-a',expectedJournalRevision:7,expectedCheckpointSeq:3,selectedEventIds:['event-1']};
  preview.select(input);preview.select({...input,selectedEventIds:['event-2']});preview.select({...input,selectedEventIds:['event-3']});assert.equal(calls.length,1);
  tasks[0].resolve(decision({selectionRequired:true}));await tick();assert.equal(calls.length,2);assert.deepEqual(calls[1].selectedEventIds,['event-3']);assert.equal(values.filter(value=>value.value).length,0);
  tasks[1].resolve(decision({selectionRequired:true,defaultEventIds:['event-1','event-2','event-3'],selectedEventIds:['event-3']}));await tick();assert.equal(values.at(-1).value.ready,true);
  preview.select(input);preview.cancel();tasks[2].resolve(decision());await tick();assert.equal(values.at(-1).value,undefined);
  checks.push('Budget preview uses host bytes and one-flight/latest selections; stale or canceled decisions cannot authorize a changed subset');
}
{
  const messages=[{id:'old-answer',role:'assistant',runId:'old-run',text:'上一轮答案'}];
  assert.equal(latestReply({run:{id:'new-run',status:'failed'},messages}).text,'');
  assert.deepEqual(latestReply({run:{id:'new-run',status:'failed'},messages,stream:{runId:'new-run',text:'半段'}}),{text:'半段',messageId:undefined,incomplete:true});
  assert.equal(latestReply({run:{id:'new-run',status:'running'},messages,stream:{runId:'old-run',text:'旧stream'}}).text,'');
  assert.equal(latestReply({messages}).text,'上一轮答案');
  checks.push('Latest interrupted round never falls back to a prior answer; transient fragments remain incomplete and cannot gain completed-message capabilities');
}

// Render actual Solid components with Solid's SSR compiler/runtime; no browser,
// Vite server, clipboard, native IPC, services or model calls.
const moduleCache=new Map();
const synthetic=values=>new SyntheticModule(Object.keys(values),function(){for(const[key,value]of Object.entries(values))this.setExport(key,value);});
async function component(file) {
  if(moduleCache.has(file))return moduleCache.get(file);
  const source=await readFile(file,'utf8');
  const transformed=transformSync(source,{filename:file.pathname,parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'ssr',hydratable:false}]],configFile:false,babelrc:false}).code;
  const module=new SourceTextModule(ts(transformed),{identifier:file.href});moduleCache.set(file,module);
  await module.link(async name=>name==='solid-js'?synthetic(solid):name==='solid-js/web'?synthetic(web):name==='lucide-solid'?synthetic({Check:()=>'',ChevronRight:()=>'',CircleAlert:()=>'',CircleHelp:()=>'',LoaderCircle:()=>'',Minus:()=>''}):name.endsWith('/i18n')?synthetic({t:value=>value}):name.endsWith('.css')?synthetic({}):name.endsWith('run-journal')?synthetic(api):component(new URL(name+(name.endsWith('.tsx')?'':'.tsx'),file)));
  await module.evaluate();return module;
}
const journalComponent=(await component(new URL('components/RunJournalDetails.tsx',import.meta.url))).namespace.default;
const noop=()=>{};
const props=extra=>({summary:summary(),state:{identity,open:true,loading:false,legacy:false,page:page()},decision:decision(),pending:false,onOpen:noop,onClose:noop,onRetry:noop,onMore:noop,onDetail:noop,onContinue:noop,onSelection:noop,...extra});
{
  const html=web.renderToString(()=>journalComponent(props({state:{identity,open:true,loading:false,legacy:false,page:page(),detail:{runId:'run-a',journalRevision:7,entry:entry(),literal:'<script>secret()</script>\n[x](https://example.test)',unavailable:null}}})));
  assert.equal(html.includes('<script>'),false);assert.ok(html.includes('&lt;script>'));assert.equal(html.includes('href='),false);assert.ok(html.includes('继续回答'));assert.equal(html.includes('type="checkbox"'),false);
  const unknown=entry('unknown',{sequence:2,phase:'unknown',selectable:false,contentBytes:0});const html2=web.renderToString(()=>journalComponent(props({decision:decision({selectionRequired:true,ready:false}),state:{identity,open:true,loading:false,legacy:false,page:page({entries:[entry(),unknown]})}})));
  assert.equal((html2.match(/type="checkbox"/g)||[]).length,1);assert.ok(html2.includes('结果未确认'));
  const legacy=web.renderToString(()=>journalComponent(props({summary:summary({canContinue:false}),state:{identity,open:true,loading:false,legacy:true}})));assert.ok(legacy.includes('未保存执行记录'));assert.equal(legacy.includes('继续回答'),false);
  checks.push('Real Solid detail component escapes payloads, has no selectable unknown metadata, and adds evidence checkboxes only after host budget rejection');
}
{
  const progress=(await component(new URL('components/ToolProgress.tsx',import.meta.url))).namespace.default;
  const html=web.renderToString(()=>progress({steps:[{id:'a',name:'a',label:'写入',status:'unknown',startedAt:1,summary:'结果未确认'}]}));
  assert.ok(html.includes('未确认'));assert.equal(html.includes('失败'),false);assert.equal(html.includes('继续回答'),false);
  const composer=await raw('components/Composer.tsx');
  const compiled=transformSync(composer,{filename:'Composer.tsx',parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'dom'}]],configFile:false,babelrc:false}).code;
  assert.ok(compiled.includes('latestReply'));assert.ok(compiled.includes('interrupted()'));assert.equal(compiled.includes('reverse().find(message => message.role'),false);
  checks.push('Actual ToolProgress separates unknown from failure; real Composer is wired to latest-run-only reply helper and interruption-only journal mounting');
}
{
  const source=await raw('components/Composer.tsx'), ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
  let expression;
  const visit=node=>{
    if(!node||typeof node!=='object')return;
    if(node.type==='JSXAttribute'&&node.name.name==='when'&&node.value?.type==='JSXExpressionContainer'){
      const value=node.value.expression, code=source.slice(value.start,value.end);
      if(code.includes("message().role === 'user'")&&code.includes('message().runId !== props.scene.run?.id'))expression=code;
    }
    for(const value of Object.values(node))if(Array.isArray(value))value.forEach(visit);else if(value&&typeof value==='object')visit(value);
  };visit(ast);assert.ok(expression);
  const visible=(messageValue,run)=>Boolean(runInNewContext(expression,{message:()=>messageValue,props:{scene:{run}}}));
  const user={role:'user',runId:'current'};
  assert.equal(visible(user,{id:'current',status:'completed'}),true,'completed current run must be available in expanded history immediately');
  for(const status of ['running','failed','canceled'])assert.equal(visible(user,{id:'current',status}),false,'current non-completed run keeps its existing live/terminal location');
  assert.equal(visible(user,{id:'next',status:'running'}),true);assert.equal(visible({role:'assistant',runId:'current'},{id:'current',status:'completed'}),false);
  checks.push('Actual Composer history predicate exposes just-completed current records without another run, while compact normal replies gain no permanent journal strip');
}
{
  let revision=7,calls=0;const reader=new JournalReader({page:async()=>{calls++;return page({summary:summary({revision})});},detail:async()=>{throw Error('unused');}},()=>{});
  reader.target(identity);await reader.open();assert.equal(calls,1);revision=8;await reader.refresh();assert.equal(calls,2);assert.equal(reader.value().page.summary.revision,8);
  reader.close();revision=9;await reader.refresh();assert.equal(calls,2);assert.equal(reader.value().page,undefined);await reader.open();assert.equal(calls,3);assert.equal(reader.value().page.summary.revision,9);
  checks.push('Run id/status pulse reloads opened journals after terminal settlement or replacement; closed history is only invalidated and ordinary snapshot revisions are not a trigger');
}
{
  // Use the real client reactive runtime and the actual component setup, not SSR
  // effects (which are intentionally no-ops) or a copied approximation of on().
  const reactive=await import(new URL('../../../node_modules/solid-js/dist/solid.js',import.meta.url));
  const calls=[],views=[],journalListeners=[];let dispose,setSnapshot,journalRevision=7,failContinue=false;
  const source=await raw('components/RunJournal.tsx');
  const transformed=transformSync(source,{filename:'RunJournal.tsx',parserOpts:{plugins:['typescript','jsx']},presets:[[solidPreset,{generate:'dom'}]],configFile:false,babelrc:false}).code;
  const mod=new SourceTextModule(ts(transformed));
  await mod.link(name=>name==='solid-js'?synthetic(reactive):name==='solid-js/web'?synthetic({createComponent:reactive.createComponent}):name.endsWith('journal-bridge')?synthetic({
    getRunJournal:async input=>{calls.push(input);return page({summary:summary({runId:input.runId,revision:journalRevision}),continuation:decision({journalRevision})});},
    getRunJournalEvent:async()=>{throw Error('unused');},previewRunContinuation:async()=>{throw Error('unused');},subscribeRunJournal:async callback=>{journalListeners.push(callback);return()=>{};},
  }):name.endsWith('run-journal')?synthetic(api):synthetic({default:props=>{views.push(props);return null;}}));
  await mod.evaluate();
  reactive.createRoot(cleanup=>{
    dispose=cleanup;const [snapshot,set]=reactive.createSignal({sceneId:'scene-a',runId:'run-a',status:'failed',revision:1,draft:'A',refs:[]});setSnapshot=set;
    for(const history of [false,true])mod.namespace.default({get sceneId(){return snapshot().sceneId;},get runId(){return history?'historical-a':snapshot().runId;},get runPulse(){return `${snapshot().runId}:${snapshot().status}`;},history,unavailable:false,onContinue:async()=>{if(failContinue)throw Error('合成继续失败');return false;}});
  });
  await tick();assert.equal(calls.length,1);assert.equal(calls[0].runId,'run-a');
  for(let i=0;i<4;i++)setSnapshot(value=>({...value,revision:value.revision+1,draft:`draft ${i}`,refs:[`ref ${i}`]}));
  await tick();assert.equal(calls.length,1,'draft/ref snapshot must not prefetch current or closed history');
  views[1].onOpen();await tick();assert.equal(calls.length,2);
  setSnapshot(value=>({...value,revision:10,draft:'new draft'}));await tick();assert.equal(calls.length,2,'opened history must not reload from unrelated snapshot');
  setSnapshot(value=>({...value,status:'canceled'}));await tick();assert.equal(calls.length,4,'terminal pulse reloads current and opened history');
  views[1].onClose();setSnapshot(value=>({...value,runId:'run-b'}));await tick();assert.equal(calls.at(-1).runId,'run-b');assert.equal(calls.filter(value=>value.runId==='historical-a').length,2,'closed history is invalidated without a read');
  views[0].onOpen();await tick();const beforeEvent=calls.length;
  journalRevision=8;journalListeners.forEach(callback=>callback({sceneId:'scene-a',runId:'run-b',revision:8}));
  assert.match(views[0].state.error,/重新载入/);await tick();assert.equal(calls.length,beforeEvent+1);assert.equal(views[0].state.page.summary.revision,8);assert.equal(views[0].state.error,undefined,'successful new page clears only its stale notice');
  failContinue=true;views[0].onContinue();await tick();assert.equal(views[0].state.error,'合成继续失败');
  journalRevision=9;journalListeners.forEach(callback=>callback({sceneId:'scene-a',runId:'run-b',revision:9}));await tick();assert.equal(views[0].state.error,'合成继续失败','refresh must not erase a real continuation failure');
  const beforeClosed=calls.length;journalListeners.forEach(callback=>callback({sceneId:'scene-a',runId:'historical-a',revision:9}));await tick();assert.equal(calls.length,beforeClosed,'closed history event stays lazy');
  dispose();const previous=calls.length;setSnapshot(value=>({...value,status:'completed'}));await tick();assert.equal(calls.length,previous);
  checks.push('Actual RunJournal client effects deduplicate unchanged identity/pulse across draft/ref snapshots; terminal changes still refresh open records and disposed effects stop');
  checks.push('Actual subscription callback clears its stale-record notice after a successful new page, preserves continuation failures and keeps closed history lazy');
}
{
  const calls=[],listeners=[],waits=[];let native=true;
  const mod=new SourceTextModule(ts(await raw('journal-bridge.ts')));
  await mod.link(name=>name.endsWith('/core')?synthetic({isTauri:()=>native,invoke:(command,args)=>{calls.push({command,args});const task=deferred();waits.push(task);return task.promise;}}):synthetic({listen:async(name,callback,options)=>{const row={name,callback,options,stopped:false};listeners.push(row);return()=>row.stopped=true;}}));await mod.evaluate();
  const bridge=mod.namespace;
  const p1=bridge.getRunJournal({...identity,limit:25}),p2=bridge.getRunJournalEvent({...identity,eventId:'event-1',expectedJournalRevision:7}),p3=bridge.previewRunContinuation({sceneId:'scene-a',sourceRunId:'run-a',expectedJournalRevision:7,expectedCheckpointSeq:3,selectedEventIds:null});
  assert.equal(calls.length,2);waits[0].resolve(page());await p1;await tick();assert.equal(calls.length,3);
  waits[1].resolve({runId:'run-a',journalRevision:7,entry:entry(),content:{type:'json',value:{text:'<svg>literal</svg>'}},references:[],binding:null,argumentsWireSha256:null});assert.equal((await p2).literal,JSON.stringify({text:'<svg>literal</svg>'},null,2));waits[2].resolve(decision());await p3;
  assert.deepEqual(calls.map(value=>value.command),['get_run_journal','get_run_journal_event','preview_run_continuation']);assert.deepEqual(calls[0].args,{sceneId:'scene-a',runId:'run-a',cursor:null,limit:25});assert.equal(Object.hasOwn(calls[2].args,'input'),false);
  let delivered=0;const stop1=await bridge.subscribeRunJournal(()=>delivered++),stop2=await bridge.subscribeRunJournal(()=>delivered++);assert.equal(listeners.length,1);assert.deepEqual(listeners[0].options,{target:'space'});listeners[0].callback({payload:{...identity,revision:8}});assert.equal(delivered,2);stop1();assert.equal(listeners[0].stopped,false);stop2();assert.equal(listeners[0].stopped,true);
  native=false;assert.equal(await bridge.getRunJournal({...identity,limit:25}),null);await assert.rejects(bridge.previewRunContinuation({}),/桌面版/);assert.equal(calls.length,3);
  checks.push('Actual bridge uses direct typed RPC args, literal content normalization, two concurrent reads, one shared space listener and honest browser refusal');
}
{
  const source=await raw('App.tsx'), ast=parse(source,{sourceType:'module',plugins:['typescript','jsx']});
  const app=ast.program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration;
  const method=app.body.body.find(node=>node.type==='FunctionDeclaration'&&node.id.name==='continueJournal');
  assert.ok(method);const executable=ts(source.slice(method.start,method.end));
  function mounted() {
    const flush=deferred(),request=deferred(),calls=[],accepted=[],errors={};
    const scene={id:'scene-a',agentId:'agent-a',connectionId:'connection-a',run:{id:'run-a',status:'failed'},draft:'A',refs:[{kind:'region',id:'r'}],closed:false,frozen:false};
    const snapshot={scenes:[scene],connections:[{id:'connection-a',revision:2}]};
    const operations=new exitApi.SpaceOperations();let busy=false,sending=false,exiting=false;
    const context={Promise,Error,ContinuationController,disposed:false,scene:()=>scene,snapshot:()=>snapshot,busy:()=>busy,sending:()=>sending,recording:()=>null,scroll:()=>null,exitPreparing:()=>exiting,spaceOperations:operations,flush:()=>flush.promise,commands:Promise.resolve(),exitFailure:undefined,
      bridge:{continueRunFromJournal:input=>{calls.push(input);return request.promise;}},accept:value=>accepted.push(value),setSending:value=>sending=value,setBusy:value=>busy=value,setSendErrors:fn=>Object.assign(errors,fn(errors))};
    const invoke=runInNewContext(`${executable}; continueJournal`,context);
    return {invoke,flush,request,calls,accepted,errors,scene,snapshot,operations,context,exit:()=>exiting=true,busy:()=>busy};
  }
  const f=mounted(),pending=f.invoke(summary(),decision());assert.equal(f.busy(),true);let drained=false;const drain=f.operations.drain(()=>true).then(()=>drained=true);await tick();assert.equal(drained,false);
  f.scene.draft='B';f.flush.resolve();await tick();assert.equal(f.calls.length,1);f.scene.draft='C';f.request.resolve({revision:10});assert.equal(await pending,true);await drain;assert.equal(f.scene.draft,'C');assert.deepEqual(f.scene.refs,[{kind:'region',id:'r'}]);assert.equal(drained,true);assert.equal(f.busy(),false);
  const second=mounted(),p=second.invoke(summary(),decision());second.exit();second.flush.resolve();assert.equal(await p,false);assert.equal(second.calls.length,0);assert.equal(second.busy(),false);
  const failed=mounted(),q=failed.invoke(summary(),decision());failed.flush.reject(Error('草稿保存失败'));assert.equal(await q,false);assert.equal(failed.scene.draft,'A');assert.equal(failed.calls.length,0);assert.equal(failed.errors['scene-a'],'草稿保存失败');
  checks.push('Actual App continuation method holds real SpaceOperations through native acknowledgement, preserves drafts/refs and denies post-exit admission or failed flush');
}
console.log(JSON.stringify({passed:checks.length,checks},null,2));
