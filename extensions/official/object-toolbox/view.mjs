// SPDX-License-Identifier: MPL-2.0
// The complete toolbox presentation and gestures live in this package.
export const html=String.raw`
<style>
*{box-sizing:border-box}body{margin:0;padding:12px;background:#f8f9fc;color:#354052;font:13px/1.5 system-ui;user-select:none}
header{display:flex;gap:8px;align-items:center;height:30px;margin-bottom:8px}header input{min-width:0;flex:1;border:0;background:none;font:inherit}
button{border:0;color:inherit;background:transparent;cursor:pointer;border-radius:10px}button:hover{background:#e8ebf3}
#grid{display:grid;grid-template-columns:repeat(3,1fr);gap:6px}
.cell{height:76px;border:1px solid #dee3ed;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:4px;touch-action:none}
.cell.over{background:#dde6ff;border-color:#6784df}.cell span{max-width:68px;text-overflow:ellipsis;overflow:hidden;white-space:nowrap;font-size:11px}
.cell img{width:64px;height:48px;object-fit:contain;pointer-events:none}.cell.moving{opacity:.35;transform:scale(.94)}.cell{transition:transform .12s,opacity .12s}.cell.folder-preview{display:grid;grid-template-columns:repeat(2,28px);gap:2px}.cell.folder-preview img{width:28px;height:25px}.cell.folder-preview span{grid-column:1/-1}
svg{width:25px;height:25px;fill:none;stroke:currentColor;stroke-width:1.6;stroke-linecap:round;stroke-linejoin:round}
footer{height:24px;display:flex;align-items:center;justify-content:center;gap:12px}#error{font-size:11px;color:#a32b3d;max-width:240px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
</style>
<header><button id="back" title="返回">‹</button><input id="title" aria-label="名称" value="工具包" readonly><span id="error" role="status"></span></header>
<div id="grid"></div><footer><button id="prev">‹</button><span id="page"></span><button id="next">›</button></footer>
<script>
let value,folder=null,page=0,busy=false,drawVersion=0;
const previews=new Map();
const grid=document.querySelector('#grid'),title=document.querySelector('#title'),error=document.querySelector('#error');
const icon=(folder,trash)=>'<svg viewBox="0 0 24 24"><path d="'+(trash?'M4 7h16 M9 3h6l1 4 M6 7l1 14h10l1-14 M10 11v6 M14 11v6':folder?'M3 6h7l2 2h9v12H3z':'M4 3h16v18H4z M7 16l4-4 6 6 M15 7h1')+'"/></svg>';
function current(){return folder?value.state.slots.find(e=>e?.id===folder)?.children:null;}
function entries(){return folder?(current()||[]).slice(page*8,page*8+8):value.state.slots;}
function targets(){mewu.setDropTargets([...grid.children].map((cell,i)=>{const r=cell.getBoundingClientRect();return {id:i===8?'trash':(folder||'root')+'/'+(folder?page*8+i:i),x:r.x,y:r.y,width:r.width,height:r.height};}));}
function draw(){
 if(!value)return;if(folder&&!current()){folder=null;page=0;}
 const version=++drawVersion;
 const all=entries();title.readOnly=!folder;title.value=folder?value.state.slots.find(e=>e?.id===folder).title:'工具包';
 grid.replaceChildren();
 for(let i=0;i<9;i++){const entry=all[i],cell=document.createElement('button');cell.className='cell';cell.dataset.slot=i;
  cell.title=i===8?'删除':entry?.title||'空位';cell.innerHTML=icon(!!entry?.children,i===8);
  if(i<8&&!entry)cell.innerHTML='';
  if(entry&&i<8){const label=document.createElement('span');label.textContent=entry.title==='对象'?(value.objects[entry.key]?.name||'对象'):entry.title;cell.title=label.textContent;cell.append(label);cell.onpointerdown=e=>drag(e,entry);
   const leaves=entry.children?entry.children.slice(0,4):[entry];
   for(const leaf of leaves)queuePreview(leaf,cell,label,version,!!entry.children);
  }
  grid.append(cell);
 }
 document.querySelector('#page').textContent=folder?(page+1)+' / '+Math.max(1,Math.ceil((current()?.length||0)/8)):'';
 document.querySelector('#prev').hidden=document.querySelector('#next').hidden=!folder;
 targets();
}
let previewQueue=Promise.resolve();
function queuePreview(entry,cell,label,version,group){
 const key=entry.key;
 const paint=image=>{if(!image||version!==drawVersion||!cell.isConnected)return;cell.querySelector('svg')?.remove();const img=document.createElement('img');img.src=image;img.alt='';img.draggable=false;cell.insertBefore(img,label);if(group)cell.classList.add('folder-preview');};
 if(previews.has(key)){queueMicrotask(()=>paint(previews.get(key)));return;}
 previewQueue=previewQueue.then(async()=>{if(version!==drawVersion)return;try{const result=await mewu.call({type:'preview',id:entry.id});previews.set(key,result.image);while(previews.size>32)previews.delete(previews.keys().next().value);paint(result.image);}catch{/* a removed/revoked object cannot expose a stale preview */}});
}
async function perform(input){if(busy)return;busy=true;error.textContent='';try{const result=await mewu.call(input);if(result?.objects)value=result;else value=await mewu.call({type:'get'});draw();}catch(e){error.textContent=e.message;}finally{busy=false;}}
async function drag(event,entry){
 if(event.button!==0||busy)return;event.preventDefault();const el=event.currentTarget,x=event.clientX,y=event.clientY,pointer=event.pointerId;
 try{
  el.classList.add('moving');
  const end=await mewu.drag(event,{image:previews.get(entry.key),label:el.title});if(end.cancelled)return;
  const px=end.x,py=end.y,moved=Math.hypot(px-x,py-y)>5;
  if(!moved){if(entry.children){folder=entry.id;page=0;draw();}else{const p=await mewu.viewportPoint(310,80);perform({type:'restore',id:entry.id,...p});}return;}
  const cell=document.elementFromPoint(px,py)?.closest('.cell'),slot=cell?Number(cell.dataset.slot):-1;
  if(slot===8)perform({type:'remove',id:entry.id});
  else if(slot>=0)perform({type:'move',id:entry.id,folder,index:folder?page*8+slot:slot});
  else if(!entry.children&&(px<0||py<0||px>innerWidth||py>innerHeight)){const p=await mewu.viewportPoint(px,py);perform({type:'restore',id:entry.id,...p});}
 }catch(e){error.textContent=e.message;}finally{el.classList.remove('moving');}
}
document.querySelector('#back').onclick=()=>{folder=null;page=0;draw();};
title.onchange=()=>{if(folder)perform({type:'rename',id:folder,title:title.value});};
document.querySelector('#prev').onclick=()=>{page=Math.max(0,page-1);draw();};
document.querySelector('#next').onclick=()=>{page=Math.min(Math.max(0,Math.ceil(current().length/8)-1),page+1);draw();};
mewu.on('change',result=>{if(result?.objects){value=result;draw();}});
mewu.on('drag',value=>{for(const cell of grid.children){const i=Number(cell.dataset.slot);cell.classList.toggle('over',value?.target===(i===8?'trash':(folder||'root')+'/'+(folder?page*8+i:i)));}});
addEventListener('resize',targets);
mewu.ready.then(()=>perform({type:'get'}));
</script>`;
