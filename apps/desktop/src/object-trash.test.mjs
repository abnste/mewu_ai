// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import test from 'node:test';
import vm from 'node:vm';
import {parse} from '@babel/parser';
const load=async path=>import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(await readFile(new URL(path,import.meta.url),'utf8'),{mode:'transform'})).toString('base64')}`);
const {objectTrash}=await load('./object-trash.ts'),{carryBlackboardObjects}=await load('./blackboard-objects.ts');
test('trash requires an active drag and current visible enabled bounds; hover and cancellation never delete',()=>{
  globalThis.innerWidth=500;globalThis.innerHeight=400;
  let box={left:300,top:340,right:348,bottom:388,width:48,height:48},disabled=false;const states=[];
  const port=objectTrash(()=>({getBoundingClientRect:()=>box}),()=>disabled,(...state)=>states.push(state));
  assert.equal(port.finish({x:320,y:360}),false);port.begin();assert.equal(port.move({x:320,y:360}),true);
  box={...box,left:350,right:398};assert.equal(port.finish({x:320,y:360}),false);
  port.begin();assert.equal(port.move({x:360,y:360}),true);port.cancel();assert.equal(port.finish({x:360,y:360}),false);
  port.begin();disabled=true;assert.equal(port.finish({x:360,y:360}),false);disabled=false;
  port.begin();assert.equal(port.move({x:NaN,y:360}),false);assert.equal(port.finish({x:360,y:360}),true);assert.deepEqual(states.at(-1),[false,false]);
  box={...box,left:510,right:558};port.begin();assert.equal(port.finish({x:520,y:360}),false);
});
test('all media are independent board objects; source-bound recording is projected without losing trim or annotations',()=>{
  const original={id:'source',background:{id:'screen',width:1000,height:500},items:['html','svg','image','video','file','text'].map((kind,i)=>({id:`item${i}`,asset:{id:`asset${i}`,name:kind==='text'?'notes.txt':kind==='file'?'document.pdf':`${kind}.data`,kind,path:`D:/assets/${i}`},x:.1,y:.2,width:.3,height:.4,state:{coordinateSpace:'background',backgroundId:'screen',blackboardSceneId:'old'},videoEdit:kind==='video'?{revision:3}:undefined,videoAnnotations:kind==='video'?{objects:[{origin:{runId:'original'}}]}:undefined}))};
  const before=structuredClone(original);let index=0;const copies=carryBlackboardObjects(original,1000,1000,()=>`copy${index++}`);
  assert.deepEqual(original,before);assert.equal(copies.length,5);const carried=original.items.filter(item=>item.asset.kind!=='file');
  for(let i=0;i<copies.length;i++){assert.equal(copies[i].y,.35);assert.equal(copies[i].height,.2);assert.notEqual(copies[i].id,carried[i].id);assert.deepEqual(copies[i].asset,carried[i].asset);assert.deepEqual(copies[i].state,{__mewuBoardSource:{sceneId:'source',itemId:carried[i].id}});}
  assert.deepEqual(copies[3].videoEdit,original.items[3].videoEdit);assert.deepEqual(copies[3].videoAnnotations,original.items[3].videoAnnotations);copies[3].videoAnnotations.objects.length=0;assert.equal(original.items[3].videoAnnotations.objects.length,1);
});
test('dropping a media object does not await its own app flush; movement still waits for persistence',async()=>{
  const source=await readFile(new URL('./components/ArtifactCard.tsx',import.meta.url),'utf8');
  const body=parse(source,{sourceType:'module',plugins:['typescript','jsx']}).program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration.body.body;
  const declaration=name=>{const node=body.find(node=>node.type==='VariableDeclaration'&&node.declarations.some(value=>value.id.name===name));return source.slice(node.start,node.end);};
  for(const remove of [true,false]){
    let flush,removed=0,updated=0,resolveUpdate;
    const listeners=new Map(),target={setPointerCapture(){},hasPointerCapture:()=>false,addEventListener:(name,handler)=>listeners.set(name,handler),removeEventListener:name=>listeners.delete(name)};
    let position={x:.1,y:.1,width:.2,height:.2};
    const props={blackboard:true,busy:false,item:{...position,asset:{id:'asset'}},trash:{begin(){},move(){},finish:()=>remove,cancel(){}},onRegisterFlush:value=>{flush=value;return()=>{};},onRemove:async()=>{await flush(()=>true);removed++;},onUpdate:async item=>{updated++;await new Promise(resolve=>{resolveUpdate=resolve;});Object.assign(props.item,item);}};
    const context=vm.createContext({props,position:()=>position,setPosition:value=>{position=value;},savedPosition:()=>({...props.item}),isImage:()=>false,isVideo:()=>false,window:{innerWidth:800,innerHeight:600},innerWidth:800,innerHeight:600,disposed:false,gestureToken:0,gestureFlight:undefined,textFlush:undefined,disposeGesture:undefined,setMoving(){},Promise,Error});
    vm.runInContext(stripTypeScriptTypes(declaration('unregisterFlush')+'\n'+declaration('gesture')+'\ngesture',{mode:'transform'}),context)({button:0,pointerId:1,currentTarget:target,clientX:100,clientY:100,preventDefault(){},stopPropagation(){}},false,true);
    listeners.get('pointermove')({clientX:140,clientY:140});
    const completion=listeners.get('pointerup')({type:'pointerup',clientX:140,clientY:140});
    if(!remove){await new Promise(resolve=>setImmediate(resolve));let flushed=false;const pending=flush(()=>true).then(()=>{flushed=true;});await new Promise(resolve=>setImmediate(resolve));assert.equal(updated,1);assert.equal(flushed,false);resolveUpdate();await pending;}
    const result=await Promise.race([completion.then(()=>true),new Promise(resolve=>setTimeout(()=>resolve(false),100))]);
    assert.equal(result,true,'removal must finish even when the app flushes the same object');assert.equal(removed,remove?1:0);assert.equal(updated,remove?0:1);
  }
});
