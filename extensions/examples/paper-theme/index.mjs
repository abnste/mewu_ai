// SPDX-License-Identifier: MPL-2.0
import {definePlugin} from '@mewu/sdk';
export default definePlugin(async ctx=>{
  const contract=await ctx.ui.themes.describe();
  if(contract.version!==1)throw Error('Host appearance contract unavailable');
  ctx.ui.themes.register({id:'paper',title:'Paper',theme:{version:1,base:'light',backgrounds:{settings:{png:"iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAIAAABvFaqvAAAANklEQVR4nGP88v4BAzUACwMDAzefKOUGMVHDNSAwahBhMBpGhMFoGBEGo2FEGIyGEWFAtTACAMc6AyOk8RrkAAAAAElFTkSuQmCC",fit:'repeat'}},colors:{foreground:'#3C372F',secondary:'#786D5D',muted:'#8C806D',surface:'#FFFDF5',surfaceSubtle:'#F5F1E5',inputBackground:'#FFFDF5',border:'#DFD6BF',accent:'#947231',accentHover:'#7E6027',accentSoft:'#F4EBCF',accentBorder:'#DCC791',settingsBackground:'#F0E9D8',settingsContent:'#FFFDF5',composerBackground:'#FFFDF5',toolbarBackground:'#FFFDF5',userMessage:'#F4EBCF',codeBackground:'#F5F1E5'}}});
});
