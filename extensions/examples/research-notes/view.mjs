// SPDX-License-Identifier: MPL-2.0
export const html=String.raw`
<style>
*{box-sizing:border-box}body{margin:0;background:#fffdf7;padding:16px;color:#423f34;font:13px system-ui}
header{font-weight:600;margin-bottom:12px}textarea{width:100%;height:160px;background:none;border:0;resize:none;font:14px/1.6 system-ui;outline:none}
#refs{min-height:100px;border-top:1px solid #e5dfd1;padding-top:10px;overflow:auto;max-height:130px}
#refs.over{background:#f2ebd8}button{border:0;background:none;color:inherit;cursor:pointer;padding:5px}
.row{display:flex;justify-content:space-between}.row button:first-child{text-align:left;max-width:250px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
#error{color:#a22;font-size:11px}
</style>
<header>研究便笺 <button id="save" style="float:right">保存</button></header>
<textarea aria-label="便笺内容" placeholder="写点什么…"></textarea><div id="refs" aria-label="参考对象"></div><span id="error" role="status"></span>
<script>
let value,busy=false;const text=document.querySelector('textarea'),refs=document.querySelector('#refs'),error=document.querySelector('#error');
function targets(){const r=refs.getBoundingClientRect();mewu.setDropTargets([{id:'references',x:r.x,y:r.y,width:r.width,height:r.height}]);}
function draw(updateText=true){
 if(updateText)text.value=value.state.text;refs.replaceChildren();
 for(const id of value.state.references){const row=document.createElement('div');row.className='row';const open=document.createElement('button'),remove=document.createElement('button');
  open.textContent=value.objects[id]?.name||'对象';remove.textContent='×';remove.title='移除引用';
  open.onclick=async()=>{const p=await mewu.viewportPoint(370,80);perform({type:'restore',id,...p},false);};
  remove.onclick=()=>perform({type:'remove',id},false);row.append(open,remove);refs.append(row);
 }
 targets();
}
async function perform(input,updateText=true){if(busy)return;busy=true;error.textContent='';try{const result=await mewu.call(input);if(result?.objects){value=result;draw(updateText);}}catch(e){error.textContent=e.message;}finally{busy=false;}}
document.querySelector('#save').onclick=()=>perform({type:'save',text:text.value});
mewu.on('change',result=>{if(result?.objects){value=result;draw(false);}});
mewu.on('drag',value=>refs.classList.toggle('over',value?.target==='references'));
addEventListener('resize',targets);mewu.ready.then(()=>perform({type:'get'}));
</script>`;
