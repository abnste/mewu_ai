// SPDX-License-Identifier: MPL-2.0
export const html=String.raw`
<style>
*{box-sizing:border-box}html,body{height:100%;width:100%;overflow:hidden}body{margin:0;background:#fffdf7;padding:20px 16px 12px;color:#423f34;font:13px system-ui;display:flex;flex-direction:column}
header{font-weight:600;margin-bottom:8px;flex:none}textarea{width:100%;flex:1;min-height:40px;background:none;border:0;resize:none;font:14px/1.6 system-ui;outline:none}
#refs{flex:none;min-height:20px;max-height:28%;border-top:1px solid #e5dfd1;padding-top:8px;overflow:auto}
#refs.over{background:#f2ebd8}button{border:0;background:none;color:inherit;cursor:pointer;padding:5px}
.row{display:flex;justify-content:space-between}.row button:first-child{text-align:left;max-width:250px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
#error{color:#a22;font-size:11px;flex:none}
</style>
<header>研究便笺 <button id="save" style="float:right">保存</button></header>
<textarea aria-label="便笺内容" placeholder="写点什么…" disabled></textarea><div id="refs" aria-label="参考对象"></div><span id="error" role="status"></span>
<script>
let value,queue=Promise.resolve(),savedText='',timer;
const text=document.querySelector('textarea'),refs=document.querySelector('#refs'),error=document.querySelector('#error');
function targets(){const r=refs.getBoundingClientRect();mewu.setDropTargets([{id:'references',x:r.x,y:r.y,width:r.width,height:r.height}]);}
function draw(updateText=true){
 if(updateText)text.value=value.state.text;refs.replaceChildren();
 for(const id of value.state.references){const row=document.createElement('div');row.className='row';const open=document.createElement('button'),remove=document.createElement('button');
  open.textContent=value.objects[id]?.name||'对象';remove.textContent='×';remove.title='移除引用';
  open.onclick=async()=>{try{const p=await mewu.viewportPoint(370,80);await perform({type:'restore',id,...p});}catch(e){error.textContent=e.message;}};
  remove.onclick=()=>void perform({type:'remove',id}).catch(()=>{});row.append(open,remove);refs.append(row);
 }
 targets();
}
function perform(input,initial=false){
 const next=queue.then(async()=>{
  error.textContent='';
  try{
   const result=await mewu.call(input);
   if(result?.objects){value=result;draw(initial);}
   if(input.type==='save')savedText=input.text;
   if(initial){savedText=text.value;text.disabled=false;}
  }catch(e){error.textContent=e.message;throw e;}
 });
 queue=next.catch(()=>{});return next;
}
async function save(){
 clearTimeout(timer);await queue;
 if(value&&text.value!==savedText)await perform({type:'save',text:text.value});
}
text.addEventListener('input',()=>{clearTimeout(timer);timer=setTimeout(()=>void save().catch(()=>{}),300);});
document.querySelector('#save').onclick=()=>void save().catch(()=>{});
mewu.onBeforeClose(async()=>{text.readOnly=true;try{await save();}catch(e){text.readOnly=false;throw e;}});
mewu.on('change',result=>{if(result?.objects){value=result;draw(false);}});
mewu.on('drag',value=>refs.classList.toggle('over',value?.target==='references'));
addEventListener('resize',targets);mewu.ready.then(()=>perform({type:'get'},true)).catch(()=>{});
</script>`;
