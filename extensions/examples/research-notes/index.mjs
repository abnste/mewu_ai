// SPDX-License-Identifier: MPL-2.0
import {definePlugin} from '@mewu/sdk';
import {html} from './view.mjs';
export default definePlugin(async ctx=>{
  const capabilities=await ctx.objects.describe();
  if(!capabilities?.capabilities?.includes('ui.objects'))throw Error('请更新 Mewu 后使用便笺');
  ctx.ui.register({id:'notes',title:'研究便笺',metadata:{surface:'space-overlay',presentation:'object'}},async input=>{
    if(input?.type==='mount')return {html,width:340,height:360};
    const library=await ctx.objects.get();
    const state=library.state??{text:'',references:[]};
    if(input?.type==='get')return {...library,state};
    let captureKey,remove=[];
    if(input?.type==='save'){
      if(typeof input.text!=='string'||input.text.length>16000)throw Error('Invalid text');
      state.text=input.text;
    }else if(input?.type==='drop'){
      if(input.target!=='references'||state.references.length>=64)throw Error('Invalid target');
      captureKey=`reference-${library.revision+1}`;state.references.push(captureKey);
    }else if(input?.type==='remove'){
      if(!state.references.includes(input.id))throw Error('Reference missing');
      remove=[input.id];state.references=state.references.filter(id=>id!==input.id);
    }else if(input?.type==='restore'){
      if(!state.references.includes(input.id))throw Error('Reference missing');
      return ctx.objects.restore(library.revision,input.id,{x:input.x,y:input.y});
    }else throw Error('Unsupported note operation');
    return ctx.objects.commit({expectedRevision:library.revision,state,remove,...(captureKey?{captureKey}:{})});
  });
});
