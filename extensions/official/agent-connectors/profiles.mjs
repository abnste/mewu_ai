// SPDX-License-Identifier: MPL-2.0
export const profiles = [
  {id:'codex',name:'Codex',protocol:'codex',args:['app-server','--listen','stdio://'],candidates:[
    'codex','${home}/.local/bin/codex','${home}/.bun/bin/codex',
    'node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe',
    'node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex/codex.exe',
    '${appData}/npm/node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex/codex.exe'
  ]},
  {id:'minimax',name:'MiniMax Code',protocol:'acp',args:['acp'],candidates:['mcode','${home}/.minimax-code/bin/mcode']},
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
  return {executable:value.executable,args:[...args],...(value.transport==='workbuddy-http'?{transport:value.transport}:{}),...(value.cwd?{cwd:value.cwd}:{}),...(Object.keys(env).length?{env}:{}),model:value.model??'',effort:value.effort??''};
}
export async function detectProfile(ctx,profile){
  const found=await ctx.processes.resolve(profile.candidates);
  if(found.executables.length||profile.id!=='workbuddy')return {executable:found.executables[0]??'',args:profile.args,platform:found.platform};
  const roots=found.platform==='windows'?['${programFiles}/WorkBuddy','${localAppData}/Programs/WorkBuddy','${localAppData}/WorkBuddy']
    :found.platform==='macos'?['/Applications/WorkBuddy.app/Contents']:[];
  if(!roots.length)return {executable:'',args:profile.args,platform:found.platform};
  const candidates=roots.map(root=>root.startsWith('/Applications')?root+'/MacOS/WorkBuddy':root+'/WorkBuddy.exe');
  const scripts=roots.flatMap(root=>['bin/codebuddy','dist/codebuddy.js'].map(name=>root+(root.startsWith('/Applications')?'/Resources':'/resources')+'/app.asar.unpacked/cli/'+name));
  const bundle=await ctx.processes.resolve(candidates,scripts);
  for(const executable of bundle.executables){
    const root=executable.replaceAll('\\','/').replace(/\/(?:MacOS\/)?WorkBuddy(?:\.exe)?$/i,'').toLowerCase();
    const script=(bundle.files??[]).find(path=>path.replaceAll('\\','/').toLowerCase().startsWith(root+'/'));
    if(script)return {executable,args:[script],transport:'workbuddy-http',env:{ELECTRON_RUN_AS_NODE:'1',CODEBUDDY_FORCE_LITE_WB_BUNDLE:'1',CODEBUDDY_CONFIG_DIR:'${home}/.workbuddy'},platform:bundle.platform};
  }
  return {executable:'',args:profile.args,platform:found.platform};
}
