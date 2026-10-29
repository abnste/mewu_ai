// SPDX-License-Identifier: MPL-2.0
import {definePlugin} from '@mewu/sdk';
export default definePlugin(async ctx=>{
  const runtime=await ctx.runtime.describe();
  if(!runtime.capabilities.includes('ui.appearance.v1'))throw Error('Host appearance contract unavailable');
  ctx.ui.themes.register({id:'aurora',title:'Aurora',theme:{version:1,base:'dark',colors:{accent:'#80D6C5',accentHover:'#A3E7DA',accentSoft:'#264740',accentBorder:'#467C70',onAccent:'#102720',workspaceBackground:'#101D23',surface:'#1B2B33',surfaceSubtle:'#233840',toolbarBackground:'#1B2B33',composerBackground:'#1B2B33',settingsBackground:'#14242B',settingsContent:'#1B2B33',userMessage:'#264740'}}});
});
