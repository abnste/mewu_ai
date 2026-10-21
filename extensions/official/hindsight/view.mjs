// SPDX-License-Identifier: MPL-2.0
export const html=String.raw`<!doctype html><meta charset="utf-8">
<style>
:root{font:12px/1.6 system-ui;color:#202632;--field:#f5f6f8;--line:#e0e3e8;--muted:#777e89;--accent:#397ee9;color-scheme:light}*{box-sizing:border-box}body{margin:0;background:transparent}h2{font-size:16px;font-weight:600;margin:0 0 24px}form{display:grid;gap:16px}label{display:grid;gap:8px;color:var(--muted);font-size:11px}input{height:36px;width:100%;padding:8px 10px;border:1px solid var(--line);border-radius:8px;background:var(--field);color:inherit;font:12px system-ui;outline:none}input:focus{border-color:var(--accent)}.actions{display:flex;align-items:center;gap:8px}button{height:32px;min-width:76px;padding:0 12px;border:1px solid var(--line);border-radius:8px;background:transparent;color:inherit;font:500 11px system-ui;cursor:pointer}button:hover{background:var(--field)}button:disabled{opacity:.45;cursor:default}#save{background:var(--accent);color:white;border-color:var(--accent)}#status{font-size:11px;color:var(--muted)}#error{color:#db5252;min-height:18px;overflow-wrap:anywhere}[hidden]{display:none!important}
</style>
<h2>Hindsight</h2>
<form id="form"><label>服务地址<input id="endpoint" type="url" required placeholder="http://127.0.0.1:8888" autocomplete="off" spellcheck="false"></label><label>API Key<input id="key" type="password" autocomplete="new-password"></label><label>记忆库前缀<input id="prefix" value="mewu" required pattern="[A-Za-z0-9_-]{1,48}" autocomplete="off" spellcheck="false"></label><div class="actions"><button id="test" type="button">测试连接</button><button id="save" type="submit">保存</button><span id="status" role="status"></span></div></form><p id="error" role="alert"></p>
<script>
const $=id=>document.getElementById(id);let data,busy=true,disposed=false,keyChanged=false;
function controls(){document.querySelectorAll('input,button').forEach(e=>e.disabled=busy);}
function show(value){data=value;$('endpoint').value=value.config?.endpoint||'';$('prefix').value=value.config?.bankPrefix||'mewu';$('key').value='';$('key').placeholder=value.secret.configured?'已保存，留空保留':'';keyChanged=false;}
$('key').oninput=()=>{keyChanged=true;};
async function perform(type){if(busy)return;busy=true;controls();$('error').textContent='';$('status').textContent=type==='test'?'检测中…':'';
try{const result=await mewu.call({type,config:{endpoint:$('endpoint').value,bankPrefix:$('prefix').value},...(type==='save'?{expected:data.config,secretRevision:data.secret.revision}:{}),...(keyChanged?{key:$('key').value}:{})});if(disposed)return;if(result.ok===false)throw Error(result.error);if(type==='save')show(result);$('status').textContent=type==='test'?'连接正常':'已保存';}catch(error){$('status').textContent='';$('error').textContent=error.message||'操作失败';}finally{busy=false;controls();}}
$('form').onsubmit=e=>{e.preventDefault();perform('save');};$('test').onclick=()=>perform('test');
mewu.on('appearance',value=>{const root=document.documentElement,c=value?.colors||{};root.style.colorScheme=value.colorScheme||'light';if(c.text)root.style.color=c.text;for(const k of ['field','line','muted','accent'])if(c[k])root.style.setProperty('--'+k,c[k]);});
controls();mewu.ready.then(()=>mewu.call({type:'get'})).then(value=>{if(disposed)return;show(value);busy=false;controls();}).catch(e=>{$('error').textContent=e.message;});addEventListener('pagehide',()=>{disposed=true;});
</script>`;
