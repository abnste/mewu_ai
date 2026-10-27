// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const state=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./extension-state.ts',import.meta.url),'utf8'),{mode:'transform'}));
await state.link(()=>{throw Error('unexpected import')});await state.evaluate();
const mod=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./toolbar-layout.ts',import.meta.url),'utf8'),{mode:'transform'}));
await mod.link(()=>state);await mod.evaluate();
const {reorderToolbar,toolbarLayoutServices}=mod.namespace;
test('partial ordering preserves node identity, omitted controls and newly added capabilities',()=>{
 const items=[{id:'draw'},{id:'copy'},{id:'export'},{id:'future'},{}];
 const sorted=reorderToolbar(items,v=>v.id,{order:['copy','unknown','draw']});
 assert.deepEqual([...sorted], [items[1],items[0],items[2],items[3],items[4]]);
 assert.deepEqual(items.map(v=>v.id),['draw','copy','export','future',undefined]);
 for(const item of items)assert.ok(sorted.includes(item));
});
test('reject duplicate IDs, unbounded and malformed layout results rather than hiding controls',()=>{
 for(const result of [null,{}, {order:'x'},{order:['copy','copy']},{order:['']},{order:[42]},{order:Array.from({length:129},(_,i)=>`${i}`)}])assert.throws(()=>reorderToolbar([],v=>v.id,result));
});
test('layout capabilities follow generic permission, service, platform and disable boundaries',()=>{
 const r={manifest:{id:'vendor.layout',permissions:['ui.render']},revision:1,state:'enabled',services:[{id:'order',kind:'service',metadata:{surface:'toolbar-layout',targets:['capture.selection']}}]};
 assert.equal(toolbarLayoutServices([r],'capture.selection').length,1);
 assert.equal(toolbarLayoutServices([r],'other').length,0);
 for(const change of [{state:'disabled'},{state:'removed'},{manifest:{...r.manifest,permissions:[]}},{unsupportedServices:{order:'not available'}},{platformError:'not available'}])assert.equal(toolbarLayoutServices([{...r,...change}],'capture.selection').length,0);
});
