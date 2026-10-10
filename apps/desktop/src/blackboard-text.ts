// SPDX-License-Identifier: MPL-2.0
import { createSignal, onCleanup } from 'solid-js';
export const MAX_BOARD_TEXT_BYTES = 2 * 1024 * 1024;
/** A single writer retains edits typed during saves and keeps failed drafts. */
export function createBlackboardText(read: (id:string)=>Promise<string>, write: (id:string,text:string)=>Promise<string>, report: (message:string)=>void) {
  const [text,setText]=createSignal(''),[loaded,setLoaded]=createSignal(false);
  let assetId='',saved='',disposed=false,loading:Promise<void>|undefined,flight:Promise<void>|undefined,timer:ReturnType<typeof setTimeout>|undefined;
  const clear=()=>{clearTimeout(timer);timer=undefined;};
  async function load(id:string) {
    if(disposed||id===assetId||flight)return;
    if(loaded()&&text()!==saved)throw Error('文本来源已变化，未保存的编辑仍保留');
    assetId=id;setLoaded(false);
    const request=(async()=>{const value=await read(id);if(!disposed&&assetId===id){saved=value;setText(value);setLoaded(true);}})();
    loading=request;
    try{await request;}finally{if(loading===request)loading=undefined;}
  }
  async function flush(active:()=>boolean=()=>true) {
    clear();if(loading)await loading;
    if(flight){await flight;return flush(active);}
    if(disposed||!active())throw Error('文本编辑已切换');
    if(!loaded())throw Error('文本尚未读取');
    // Claim the writer before invoking a bridge that can publish synchronously.
    const request=Promise.resolve().then(async()=>{while(text()!==saved){
      if(disposed||!active())throw Error('文本编辑已切换');
      const value=text(),expected=assetId;
      const next=await write(expected,value);if(!next||next===expected)throw Error('文本保存结果无效');assetId=next;saved=value;
    }});
    flight=request;
    try{await request;}finally{if(flight===request)flight=undefined;}
  }
  const schedule=()=>{clear();timer=setTimeout(()=>{timer=undefined;void flush().catch(error=>report(error instanceof Error?error.message:String(error)));},450);};
  function edit(value:string,composing=false){
    if(disposed||!loaded())return;
    if(new TextEncoder().encode(value).length>MAX_BOARD_TEXT_BYTES||value.includes('\0')){report('文本超出限制或包含无效字符');return;}
    setText(value);if(!composing)schedule();
  }
  onCleanup(()=>{disposed=true;clear();});
  return {text,loaded,load,edit,flush,schedule};
}
export type ResizeCorner='nw'|'ne'|'sw'|'se';
export function resizeBoardObject(start:{x:number;y:number;width:number;height:number},dx:number,dy:number,corner:ResizeCorner,vw:number,vh:number){
  const left=corner.endsWith('w'),top=corner.startsWith('n'),minW=Math.min(96/vw,start.width),minH=Math.min(64/vh,start.height);
  const x=left?Math.max(0,Math.min(start.x+start.width-minW,start.x+dx)):start.x;
  const y=top?Math.max(0,Math.min(start.y+start.height-minH,start.y+dy)):start.y;
  return {x,y,width:left?start.x+start.width-x:Math.max(minW,Math.min(1-x,start.width+dx)),height:top?start.y+start.height-y:Math.max(minH,Math.min(1-y,start.height+dy))};
}
