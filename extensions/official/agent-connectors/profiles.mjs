// SPDX-License-Identifier: MPL-2.0
export const profiles = [
  {id:'codex',name:'Codex',protocol:'codex',args:['app-server','--listen','stdio://'],candidates:[
    'codex','${home}/.local/bin/codex','${home}/.bun/bin/codex','/opt/homebrew/bin/codex','/usr/local/bin/codex','${home}/.npm-global/bin/codex',
    'node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe',
    'node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex/codex.exe',
    '${appData}/npm/node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex/codex.exe'
  ]},
  {id:'minimax',name:'MiniMax Code',protocol:'acp',args:['acp'],candidates:['mcode','${home}/.minimax-code/bin/mcode','${home}/.local/bin/mcode','/opt/homebrew/bin/mcode','/usr/local/bin/mcode','${home}/.npm-global/bin/mcode']},
  {id:'workbuddy',name:'WorkBuddy / CodeBuddy',protocol:'acp',args:['--acp'],candidates:['codebuddy','workbuddy','${home}/.local/bin/codebuddy']},
  {id:'hermes',name:'Hermes',protocol:'acp',args:['acp'],candidates:['hermes','${home}/.local/bin/hermes','${home}/.hermes/hermes-agent/.venv/bin/hermes']}
];
export function validateConfig(value,profile){
  if(!value||typeof value!=='object'||Array.isArray(value))throw Error('连接配置无效');
  const text=(v,max)=>typeof v==='string'&&v.length<=max&&!/[\u0000-\u001f]/.test(v);
  if(!text(value.executable,4096)||!value.executable.trim())throw Error('请选择已安装的运行程序');
  const args=value.args??profile.args;
  if(!Array.isArray(args)||args.length>64||!args.every(v=>text(v,4096)))throw Error('启动参数需要填写 JSON 字符串数组');
  if(!text(value.cwd??'',4096)||!text(value.model??'',300)||!text(value.effort??'',80))throw Error('连接选项无效');
  const env=value.env??{};
  if(!env||typeof env!=='object'||Array.isArray(env)||Object.keys(env).length>64||Object.entries(env).some(([key,v])=>!/^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(key)||!text(v,16384)))throw Error('环境变量需要填写 JSON 字符串对象');
  if(value.transport!==undefined&&!['stdio','workbuddy-http'].includes(value.transport)||value.transport==='workbuddy-http'&&profile.id!=='workbuddy')throw Error('连接方式无效');
  if(value.launchMode!==undefined&&!['auto','manual'].includes(value.launchMode))throw Error('程序定位方式无效');
  return {executable:value.executable,args:[...args],...(value.launchMode?{launchMode:value.launchMode}:{}),...(value.transport==='workbuddy-http'?{transport:value.transport}:{}),...(value.cwd?{cwd:value.cwd}:{}),...(Object.keys(env).length?{env}:{}),model:value.model??'',effort:value.effort??''};
}
const scriptPath=value=>value.startsWith('\\\\?\\')?value.slice(4):value;
async function nodeLauncher(ctx,profile,platform){
  const nodes=await ctx.processes.resolve(['node','${programFiles}/nodejs/node.exe','${home}/.minimax-code/node/node.exe','/opt/homebrew/bin/node','/usr/local/bin/node','/usr/bin/node'].filter(p=>platform==='windows'?!p.startsWith('/'):!p.includes('${programFiles}')));
  const suffix=profile.id==='codex'?'@openai/codex/bin/codex.js':profile.id==='minimax'?'@minimax-ai/code/cli.js':null;
  if(!suffix)return null;
  for(const node of nodes.executables){
    const parent=scriptPath(node).replaceAll('\\','/').replace(/\/[^/]+$/,'');
    const files=[`${parent}/node_modules/${suffix}`,...(platform==='windows'?[]:[`${parent}/../lib/node_modules/${suffix}`])];
    if(platform==='windows')files.push('${appData}/npm/node_modules/'+suffix,'${home}/.minimax-code/node_modules/'+suffix);
    else files.push(`/opt/homebrew/lib/node_modules/${suffix}`,`/usr/local/lib/node_modules/${suffix}`,'${home}/.minimax-code/node_modules/'+suffix);
    const found=await ctx.processes.resolve([],files);
    if(found.files?.length)return {executable:node,args:[scriptPath(found.files[0]),...profile.args],platform,launchMode:'auto'};
  }
  return null;
}
export async function detectProfile(ctx,profile){
  let found=await ctx.processes.resolve(profile.candidates.filter(p=>!p.startsWith('/')));
  if(!found.executables.length&&found.platform!=='windows'){
    const unix=profile.candidates.filter(p=>p.startsWith('/'));
    if(unix.length)found=await ctx.processes.resolve(unix);
  }
  if(found.executables.length)return {executable:found.executables[0],args:profile.args,platform:found.platform,launchMode:'auto'};
  if(profile.id==='codex'&&found.platform==='windows'&&(await ctx.runtime.describe()).capabilities.includes('process.installation-search')){
    const desktop=await ctx.processes.resolve(['${localAppData}/OpenAI/Codex/bin/codex.exe','${localAppData}/OpenAI/ChatGPT/bin/codex.exe'],[],['Codex','ChatGPT'].map(product=>({root:'${localAppData}/OpenAI/'+product+'/bin',relativePath:'codex.exe'})));
    if(desktop.executables.length)return {executable:desktop.executables[0],args:profile.args,platform:found.platform,launchMode:'auto'};
  }
  if(['codex','minimax'].includes(profile.id)){
    const launch=await nodeLauncher(ctx,profile,found.platform);if(launch)return launch;
  }
  if(profile.id!=='workbuddy')return {executable:'',args:profile.args,platform:found.platform};
  const roots=found.platform==='windows'?['${programFiles}/WorkBuddy','${localAppData}/Programs/WorkBuddy','${localAppData}/WorkBuddy']
    :found.platform==='macos'?['/Applications/WorkBuddy.app/Contents']:[];
  if(!roots.length)return {executable:'',args:profile.args,platform:found.platform};
  const candidates=roots.flatMap(root=>root.startsWith('/Applications')?[root+'/MacOS/WorkBuddy',root+'/MacOS/Electron']:[root+'/WorkBuddy.exe']);
  const scripts=roots.flatMap(root=>['bin/codebuddy','dist/codebuddy.js'].map(name=>root+(root.startsWith('/Applications')?'/Resources':'/resources')+'/app.asar.unpacked/cli/'+name));
  const bundle=await ctx.processes.resolve(candidates,scripts);
  for(const executable of bundle.executables){
    const root=executable.replaceAll('\\','/').replace(/\/(?:MacOS\/(?:WorkBuddy|Electron)|WorkBuddy\.exe)$/i,'').toLowerCase();
    const script=(bundle.files??[]).find(path=>path.replaceAll('\\','/').toLowerCase().startsWith(root+'/'));
    if(script)return {executable,args:[script],transport:'workbuddy-http',env:{ELECTRON_RUN_AS_NODE:'1',CODEBUDDY_FORCE_LITE_WB_BUNDLE:'1',CODEBUDDY_CONFIG_DIR:'${home}/.workbuddy'},platform:bundle.platform};
  }
  return {executable:'',args:profile.args,platform:found.platform};
}
export async function resolveConfig(ctx,config,profile){
  if(config.launchMode!=='auto')return config;
  const found=await detectProfile(ctx,profile);
  if(!found.executable)throw Error('未找到已安装的官方客户端，请在外部 Agent 设置中重新检测');
  return {...config,executable:found.executable,args:found.args,...(found.transport?{transport:found.transport}:{}),env:{...config.env,...found.env}};
}
