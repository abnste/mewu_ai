// SPDX-License-Identifier: MPL-2.0
import {definePlugin} from '@mewu/sdk';
import {html} from './view.mjs';
const root=()=>Array(8).fill(null);
function locate(slots,id) {
  for(let i=0;i<slots.length;i++){
    const value=slots[i];if(value?.id===id)return {list:slots,index:i,value};
    if(value?.children){const found=locate(value.children,id);if(found)return found;}
  }
}
function destination(slots,folder,index) {
  if(!Number.isInteger(index)||index<0||index>255)throw Error('Invalid slot');
  if(folder){const entry=locate(slots,folder)?.value;if(!entry?.children)throw Error('Folder missing');return {list:entry.children,index:Math.min(index,entry.children.length)};}
  if(index>=8)throw Error('Invalid root slot');
  return {list:slots,index};
}
function put(target,item,revision) {
  const old=target.list[target.index];
  if(!old){target.list[target.index]=item;return;}
  if(old.children){if(item.children)throw Error('Nested folders are not supported');if(old.children.length>=256)throw Error('Folder full');old.children.push(item);return;}
  if(item.children)throw Error('Drop folders on an empty slot');
  target.list[target.index]={id:`folder-${revision+1}`,title:'文件夹',children:[old,item]};
}
export default definePlugin(async ctx=>{
  const capabilities=await ctx.objects.describe();
  if(!capabilities?.capabilities?.includes('objects.preview'))throw Error('请更新 Mewu 后使用工具包');
  ctx.ui.register({id:'toolbox',title:'工具包',metadata:{surface:'space-overlay'}},async input=>{
    if(input?.type==='mount')return {html,width:268,height:328};
    const value=await ctx.objects.get();
    const slots=value.state?.slots??root();
    const present=()=>({...value,state:{slots}});
    if(input?.type==='get')return present();
    if(input?.type==='preview'){
      const entry=locate(slots,input.id)?.value;
      if(!entry?.key)throw Error('Object missing');
      return ctx.objects.preview(value.revision,entry.key);
    }
    if(input?.type==='drop'&&input.target==='trash')return {removeSource:true};
    if(input?.type==='restore'){
      const entry=locate(slots,input.id)?.value;
      if(!entry?.key)throw Error('Object missing');
      return ctx.objects.restore(value.revision,entry.key,{x:input.x,y:input.y});
    }
    let captureKey,remove=[];
    if(input?.type==='drop'){
      const [folder,index]=String(input.target).split('/');
      captureKey=`object-${value.revision+1}`;
      put(destination(slots,folder==='root'?null:folder,Number(index)),
        {id:captureKey,key:captureKey,title:'对象'},value.revision);
    }else if(input?.type==='move'){
      const found=locate(slots,input.id);if(!found)throw Error('Object missing');
      const target=destination(slots,input.folder, input.index);
      if(target.list===found.list&&target.index===found.index)return present();
      if(found.value.children?.some(e=>e.id===input.folder)||found.value.id===input.folder)throw Error('Cannot move into itself');
      if(found.list===slots)found.list[found.index]=null;else found.list.splice(found.index,1);
      put(destination(slots,input.folder,input.index),found.value,value.revision);
    }else if(input?.type==='remove'){
      const found=locate(slots,input.id);if(!found)throw Error('Object missing');
      remove=found.value.children?found.value.children.map(e=>e.key):[found.value.key];
      if(found.list===slots)found.list[found.index]=null;else found.list.splice(found.index,1);
    }else if(input?.type==='rename'){
      const found=locate(slots,input.id);if(!found||typeof input.title!=='string'||!input.title.trim()||input.title.length>120)throw Error('Invalid name');
      found.value.title=input.title.trim();
    }else throw Error('Unsupported toolbox operation');
    return ctx.objects.commit({expectedRevision:value.revision,state:{slots},remove,...(captureKey?{captureKey}:{})});
  });
});
