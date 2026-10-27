// SPDX-License-Identifier: MPL-2.0
export const html=String.raw`
<meta name="color-scheme" content="light dark">
<style>
:root{font:12px/1.5 system-ui;color:#292d35;--field:#f4f5f7;--border:#e0e3e8;--muted:#777e89;--accent:#397ee9;--accentText:#fff;color-scheme:light}
*{box-sizing:border-box}body{margin:0;background:transparent}h2{margin:0 0 20px;font-size:16px;font-weight:600}
nav{display:flex;gap:4px;margin-bottom:24px;padding:3px;background:var(--field);border-radius:9px;width:fit-content;max-width:100%;flex-wrap:wrap}
button{height:32px;min-width:76px;padding:0 12px;border:1px solid var(--border);border-radius:8px;background:transparent;color:inherit;font:500 11px system-ui;cursor:pointer;white-space:nowrap}
button:hover{background:var(--field)}button:disabled{opacity:.45;cursor:default}nav button{border:0;min-width:0;height:28px}nav button[aria-selected=true]{background:var(--accent);color:var(--accentText)}
.connection-form{display:grid;gap:16px}label{display:grid;gap:8px;color:var(--muted);font-size:11px}.input-row{display:flex;gap:8px}input,textarea,select{width:100%;min-width:0;padding:8px 10px;border:1px solid var(--border);border-radius:8px;background:var(--field);color:inherit;font:12px/18px system-ui;outline:none}input{height:36px}textarea{font-family:ui-monospace,monospace;resize:vertical;min-height:64px}input:focus,textarea:focus,select:focus{border-color:var(--accent)}
.input-row input{flex:1}.input-row button{height:36px}.actions{display:flex;gap:8px;margin-top:4px;align-items:center}#save{background:var(--accent);color:var(--accentText);border-color:var(--accent)}#status{font-size:11px;color:var(--muted);white-space:pre-wrap;overflow-wrap:anywhere}.help{color:var(--muted);font-size:11px;margin:12px 0 0}details{border-top:1px solid var(--border);padding-top:12px}summary{cursor:pointer;color:var(--muted);margin-bottom:12px}#error{color:#db5252;min-height:18px;white-space:pre-wrap}select{height:36px}
.model-field{position:relative}.model-field>input{padding-right:38px}.model-field>button{position:absolute;right:2px;top:2px;width:32px;height:32px;min-width:0;padding:8px;border:0}.model-options{position:absolute;z-index:2;left:0;right:0;top:40px;max-height:210px;overflow:auto;background:var(--field);border:1px solid var(--border);border-radius:8px;padding:4px;box-shadow:0 8px 24px #0002}.model-option{padding:7px 8px;border-radius:5px;cursor:pointer;font-size:12px}.model-option:hover,.model-option[aria-selected=true]{background:var(--accent);color:white}[hidden]{display:none!important}
.transport-options{display:flex;gap:8px}.transport-options button[aria-pressed=true]{color:var(--accent);border-color:var(--accent)}
</style>
<h2>外部 Agent</h2><nav role="tablist" aria-label="Agent 平台"></nav>
<section class="connection-form">
  <label>运行程序<div class="input-row"><input id="executable" autocomplete="off" spellcheck="false" placeholder="本机可执行文件的完整路径"><button id="detect" type="button">自动检测</button></div></label>
  <div id="transport-field" hidden><label>连接方式<div class="transport-options"><button id="transport-stdio" type="button" aria-pressed="true">ACP CLI</button><button id="transport-desktop" type="button" aria-pressed="false">桌面内置 ACP</button></div></label></div>
  <label>模型<div class="model-field" id="model-field"><input id="model" role="combobox" aria-autocomplete="list" aria-controls="models" aria-expanded="false" autocomplete="off" placeholder="使用客户端默认模型"><button id="choose-model" type="button" aria-label="选择模型"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg></button><div id="models" class="model-options" role="listbox" hidden></div></div></label>
  <details><summary>启动选项</summary><label>启动参数<textarea id="args" spellcheck="false" aria-label="启动参数"></textarea></label><label style="margin-top:12px">环境变量<textarea id="env" spellcheck="false" aria-label="环境变量"></textarea></label><label style="margin-top:12px">工作目录<input id="cwd" placeholder="使用插件工作目录" spellcheck="false"></label><label id="effort-field" style="margin-top:12px">思考程度<input id="effort" placeholder="使用客户端默认设置"></label></details>
  <div class="actions"><button id="test" type="button">测试连接</button><button id="save" type="button">保存</button><span id="status" role="status"></span></div>
</section>
<div id="error" role="alert"></div><p class="help" id="hint"></p>
<script>
const $=id=>document.getElementById(id);let data,selected='codex',busy=true,disposed=false,transport='stdio',launchMode='manual';
const drafts=new Map();
let models=[],modelIndex=-1,allModels=false;
const modelValues=()=>{const query=allModels?'':$('model').value.toLowerCase();return models.filter(m=>(m.name+' '+m.id).toLowerCase().includes(query));};
function hideModels(){$('models').hidden=true;$('model').setAttribute('aria-expanded','false');modelIndex=-1;allModels=false;}
function drawModels(){const values=modelValues();$('models').replaceChildren(...values.map((m,index)=>{const option=document.createElement('div');option.className='model-option';option.role='option';option.setAttribute('aria-selected',String(modelIndex===index));option.textContent=m.name===m.id?m.id:m.name+' · '+m.id;option.onpointerdown=e=>{e.preventDefault();$('model').value=m.id;hideModels();};return option;}));$('models').hidden=!values.length;$('model').setAttribute('aria-expanded',String(values.length>0));}
$('choose-model').onclick=()=>{if($('models').hidden){allModels=true;modelIndex=models.findIndex(m=>m.id===$('model').value);$('model').focus();drawModels();}else hideModels();};$('model').oninput=()=>{allModels=false;modelIndex=-1;drawModels();};
$('model').onkeydown=e=>{if(e.key==='Escape'&&!$('models').hidden){e.preventDefault();e.stopPropagation();hideModels();}if(e.key==='ArrowDown'||e.key==='ArrowUp'){e.preventDefault();if($('models').hidden)allModels=true;const count=modelValues().length;modelIndex=count?(modelIndex<0?(e.key==='ArrowDown'?0:count-1):(modelIndex+(e.key==='ArrowDown'?1:-1)+count)%count):-1;drawModels();}if(e.key==='Enter'&&!$('models').hidden&&modelIndex>=0){e.preventDefault();$('model').value=modelValues()[modelIndex].id;hideModels();}};
document.addEventListener('pointerdown',e=>{if(!$('model-field').contains(e.target))hideModels();});
function config(){const args=JSON.parse($('args').value||'[]'),env=JSON.parse($('env').value||'{}');if(!Array.isArray(args)||!args.every(v=>typeof v==='string'))throw Error('启动参数需要填写 JSON 字符串数组');if(!env||Array.isArray(env)||typeof env!=='object'||!Object.values(env).every(v=>typeof v==='string'))throw Error('环境变量需要填写 JSON 字符串对象');return{executable:$('executable').value.trim(),args,launchMode,...(selected==='workbuddy'&&transport==='workbuddy-http'?{transport}:{}),...(Object.keys(env).length?{env}:{}),model:$('model').value.trim(),cwd:$('cwd').value.trim(),effort:$('effort').value.trim()};}
$('executable').oninput=$('args').oninput=()=>{launchMode='manual';};
function drawTransport(){$('transport-field').hidden=selected!=='workbuddy';$('transport-stdio').setAttribute('aria-pressed',String(transport==='stdio'));$('transport-desktop').setAttribute('aria-pressed',String(transport==='workbuddy-http'));}
$('transport-stdio').onclick=()=>{transport='stdio';drawTransport();};$('transport-desktop').onclick=()=>{transport='workbuddy-http';drawTransport();};
function controls(){for(const button of document.querySelectorAll('button,input,textarea'))button.disabled=busy;$('choose-model').disabled=busy||models.length===0;}
function render(){
 const profile=data.profiles.find(p=>p.id===selected),value=drafts.get(selected)||data.connections[selected]||{args:profile.args};
 $('executable').value=value.executable||'';$('args').value=JSON.stringify(value.args||profile.args);$('env').value=JSON.stringify(value.env||{});$('model').value=value.model||'';$('cwd').value=value.cwd||'';$('effort').value=value.effort||'';
 transport=value.transport||'stdio';launchMode=value.launchMode||'manual';drawTransport();
 $('effort-field').hidden=selected!=='codex';models=[];hideModels();$('models').replaceChildren();$('status').textContent='';$('error').textContent='';
 $('hint').textContent=selected==='minimax'?'使用已安装并登录的 MiniMax Code CLI（mcode acp）。':selected==='workbuddy'?'自动检测可接入 WorkBuddy 桌面内置 ACP，也可指定 CodeBuddy CLI。':selected==='hermes'?'使用已配置 ACP 支持的 Hermes；记忆与技能沿用 Hermes 的设置。':'使用本机 Codex 登录；无需复制 API Key。';
 document.querySelector('nav').replaceChildren(...data.profiles.map(p=>{const b=document.createElement('button');b.type='button';b.role='tab';b.textContent=p.name;b.setAttribute('aria-selected',String(selected===p.id));b.onclick=()=>{try{drafts.set(selected,config());selected=p.id;render();}catch(e){$('error').textContent=e.message;}};return b;}));controls();
}
async function perform(type){
 if(busy)return;busy=true;controls();$('error').textContent='';$('status').textContent=type==='test'?'检测中…':'';
 try{
   const result=await mewu.call({type,provider:selected,...(type==='detect'?{}:{config:config(),expected:data.connections[selected]||null})});
   if(disposed)return;
   if(type==='detect'){if(!result.executable)throw Error('未找到可用程序，请先安装官方 CLI，或填写运行程序及启动参数。');$('executable').value=result.executable;$('args').value=JSON.stringify(result.args);$('env').value=JSON.stringify(result.env||{});transport=result.transport||'stdio';launchMode=result.launchMode||'manual';drawTransport();$('status').textContent='已找到';}
   if(type==='test'){if(!result.ok)throw Error(result.error||'连接失败');models=result.models.map(m=>({id:m.id,name:m.name||m.id}));if(!$('model').value&&result.models.some(m=>m.default))$('model').value=result.models.find(m=>m.default).id;$('status').textContent='服务可连接'+(result.image?' · 支持图片':'');}
   if(type==='save'){data.connections[selected]=result.config;drafts.delete(selected);$('status').textContent='已保存';}
 }catch(e){$('status').textContent='';$('error').textContent=e.message||'操作失败';}finally{busy=false;controls();}
}
$('detect').onclick=()=>perform('detect');$('test').onclick=()=>perform('test');$('save').onclick=()=>perform('save');
controls();mewu.on('appearance',value=>{const root=document.documentElement,colors=value?.colors||{};root.style.colorScheme=value.colorScheme||'light';if(colors.text)root.style.color=colors.text;for(const [key,name] of [['field','--field'],['line','--border'],['muted','--muted'],['accent','--accent']])if(colors[key])root.style.setProperty(name,colors[key]);});
mewu.ready.then(()=>mewu.call({type:'get'})).then(value=>{data=value;busy=false;render();}).catch(e=>{$('error').textContent=e.message;});
addEventListener('pagehide',()=>{disposed=true;});
</script>`;
