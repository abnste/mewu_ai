// Exercise the actual Composer event handlers with a controlled clock and DOM bounds.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import { stripTypeScriptTypes } from 'node:module';
import { parse } from '@babel/parser';
import test from 'node:test';

const source = await readFile(new URL('components/Composer.tsx', import.meta.url), 'utf8');
const component = parse(source,{sourceType:'module',plugins:['typescript','jsx']}).program.body.find(n=>n.type==='ExportDefaultDeclaration').declaration;
const declarations = component.body.body.flatMap(n=>n.type==='VariableDeclaration'?n.declarations:[]);
const handler = name => {
  const value = declarations.find(n=>n.id.name===name).init;
  return stripTypeScriptTypes(`const ${name} = ${source.slice(value.start,value.end)};`,{mode:'transform'});
};

function fixture(overrides={}, deferredFocus=false) {
  let clock=0, nextId=0;
  const timers=new Map(), focus=[], blurred=[], microtasks=[];
  class Element { constructor(selector=''){this.selector=selector;} closest(selector){return selector.split(',').includes(this.selector)?this:null;} }
  const state={hidden:false,position:undefined,dragging:false,picker:undefined,voiceMenu:false,...overrides};
  const props={selectionActive:false,closing:false,voice:undefined};
  const activeElement={blur:()=>blurred.push(true)};
  const shell={offsetWidth:574,getBoundingClientRect:()=>state.bounds??({left:225,right:799,top:490,bottom:576}),contains:()=>true};
  const actual=runInNewContext(`
    let hideTimer, composing=false, pointerHeld=false, pointerOutsideWindow=false, lastPointer={x:0,y:0}, focusAnchor, focusVersion=0;
    ${['cancelHide','hideBlocked','pointerInDock','scheduleHide','reveal','pointer','pointerExit','selectionStarted'].map(handler).join('\n')}
    ({pointer,pointerExit,reveal,cancelHide,selectionStarted,anchor:v=>focusAnchor=v,composing:v=>composing=v});
  `,{Element,props,shell,document:{activeElement},innerWidth:1024,innerHeight:600,
    input:{focus:()=>focus.push(true),blur:()=>blurred.push(true)},queueMicrotask:fn=>deferredFocus?microtasks.push(fn):fn(),
    hidden:()=>state.hidden,setHidden:v=>state.hidden=v,position:()=>state.position,dragging:()=>state.dragging,
    picker:()=>state.picker,voiceMenu:()=>state.voiceMenu,
    setTimeout:(fn,delay)=>{const id=++nextId;timers.set(id,{fn,at:clock+delay});return id;},clearTimeout:id=>timers.delete(id)
  });
  const move=(x,y,extra={})=>actual.pointer({clientX:x,clientY:y,buttons:0,pointerType:'mouse',target:new Element(),...extra});
  const tick=ms=>{clock+=ms;for(const [id,t] of [...timers])if(t.at<=clock){timers.delete(id);t.fn();}};
  return {actual,state,props,move,tick,focus,blurred,timers,Element,flushFocus:()=>microtasks.splice(0).forEach(fn=>fn())};
}

test('moving onto ordinary blank space retracts the dock after grace, independent of capture-region hit tests',()=>{
  const f=fixture();f.move(100,100);f.tick(239);assert.equal(f.state.hidden,false);
  f.tick(1);assert.equal(f.state.hidden,true);assert.equal(f.blurred.length,1);
  f.move(500,200);f.tick(300);assert.equal(f.state.hidden,true,'moving over blank space cannot reopen the bar');
});
test('returning to the bar cancels a queued hide and the bottom edge reveals without stealing keyboard focus',()=>{
  const f=fixture();f.move(100,100);f.tick(100);f.move(500,530);f.tick(200);assert.equal(f.state.hidden,false);
  f.move(100,100);f.tick(240);assert.equal(f.state.hidden,true);
  f.move(500,590);assert.equal(f.state.hidden,false);assert.equal(f.focus.length,0);
  f.actual.reveal(true);assert.equal(f.focus.length,1,'only explicit keyboard reveal requests focus');
});
test('menus, recording speech, dragging, IME, detached windows and held buttons preserve active interaction',()=>{
  for(const protect of ['picker','voiceMenu','dragging','position','voice','composing','buttons','closing']){
    const f=fixture();
    if(protect==='voice')f.props.voice={pending:true};
    else if(protect==='composing')f.actual.composing(true);
    else if(protect==='closing')f.props.closing=true;
    else if(protect!=='buttons')f.state[protect]=protect==='position'?{x:20,y:30}:true;
    f.move(100,100,protect==='buttons'?{buttons:1}:{});f.tick(500);
    assert.equal(f.state.hidden,false,protect);
  }
});
test('late hide rechecks current menu and composition state instead of concealing newly opened content',()=>{
  for(const protect of ['picker','composing']){
    const f=fixture();f.move(100,100);
    if(protect==='picker')f.state.picker='model';else f.actual.composing(true);
    f.tick(240);assert.equal(f.state.hidden,false);
  }
});
test('selection drag is protected until release and touch without hover cannot hide the keyboard controls',()=>{
  const f=fixture();f.move(100,100,{buttons:1});f.tick(500);assert.equal(f.state.hidden,false);
  f.move(100,100);f.tick(240);assert.equal(f.state.hidden,true);
  const touch=fixture();touch.move(100,100,{pointerType:'touch'});touch.tick(500);assert.equal(touch.state.hidden,false);
});
test('explicit input request remains visible until intentional pointer movement, then normal auto-hide resumes',()=>{
  const f=fixture();f.actual.anchor({x:10,y:10});f.move(12,11);f.tick(300);assert.equal(f.state.hidden,false);
  f.move(100,100);f.tick(240);assert.equal(f.state.hidden,true);
});
test('viewport leave schedules hide, and teardown cancels its callback',()=>{
  const f=fixture();f.actual.pointerExit({relatedTarget:null,pointerType:'mouse'});f.tick(240);assert.equal(f.state.hidden,true);
  const disposed=fixture();disposed.move(100,100);disposed.actual.cancelHide();disposed.tick(500);assert.equal(disposed.state.hidden,false);
});
test('an expanding reply or completed reveal animation keeps the bar under a stationary pointer',()=>{
  const f=fixture();f.move(500,400);
  f.state.bounds={left:225,right:799,top:350,bottom:576};
  f.tick(240);assert.equal(f.state.hidden,false,'timer must recheck current layout without another pointermove');
  f.actual.pointerExit({relatedTarget:null,pointerType:'mouse'});f.tick(240);
  assert.equal(f.state.hidden,true,'leaving the window must not use stale in-bar coordinates');
});
test('a queued explicit focus cannot steal focus after the bar hides, selection starts, or closing begins',()=>{
  for(const change of ['hidden','selection','closing','newerReveal']){
    const f=fixture({},true);f.actual.reveal(true);
    if(change==='hidden'){f.move(100,100);f.tick(240);}
    else if(change==='selection')f.props.selectionActive=true;
    else if(change==='closing')f.props.closing=true;
    else f.actual.reveal(false);
    f.flushFocus();assert.equal(f.focus.length,0,change);
  }
});

test('selection releases keyboard ownership even when the composer was dragged out of the dock',()=>{
  for(const position of [undefined,{x:20,y:30}]){
    const f=fixture({position},true);f.props.draft='未发送草稿';f.actual.reveal(true);
    f.props.selectionActive=true;f.actual.selectionStarted();f.flushFocus();
    assert.equal(f.blurred.length,1,'a retained textarea focus must not turn screenshot Enter into a send');
    assert.equal(f.focus.length,0,'a queued reveal cannot reclaim keyboard ownership');
    assert.equal(f.state.hidden,!position,'a detached composer remains visible');
    assert.equal(f.props.draft,'未发送草稿');
  }
});
