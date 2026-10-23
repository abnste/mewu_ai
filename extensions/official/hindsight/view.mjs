// SPDX-License-Identifier: MPL-2.0
export const html=String.raw`<!doctype html><meta charset="utf-8">
<style>
:root{font:12px/1.6 system-ui;color:#202632;--field:#f5f6f8;--line:#e0e3e8;--muted:#777e89;--accent:#397ee9;color-scheme:light}*{box-sizing:border-box}body{margin:0;background:transparent}h2{font-size:16px;font-weight:600;margin:0 0 24px}form{display:grid;gap:16px}label{display:grid;gap:8px;color:var(--muted);font-size:11px}input{height:36px;width:100%;padding:8px 10px;border:1px solid var(--line);border-radius:8px;background:var(--field);color:inherit;font:12px system-ui;outline:none}input:focus{border-color:var(--accent)}.toggle{display:flex;align-items:center;justify-content:space-between;font-size:12px;color:inherit}.toggle input{appearance:none;width:32px;height:18px;border-radius:9px;padding:2px;margin:0;cursor:pointer;flex:none}.toggle input::before{content:"";display:block;width:12px;height:12px;border-radius:50%;background:var(--muted);transition:transform .12s}.toggle input:checked{background:var(--accent);border-color:var(--accent)}.toggle input:checked::before{background:white;transform:translateX(14px)}.actions{display:flex;align-items:center;gap:8px}button{height:32px;min-width:76px;padding:0 12px;border:1px solid var(--line);border-radius:8px;background:transparent;color:inherit;font:500 11px system-ui;cursor:pointer}button:hover{background:var(--field)}button:disabled{opacity:.45;cursor:default}#save{background:var(--accent);color:white;border-color:var(--accent)}#status{font-size:11px;color:var(--muted)}#error{color:#db5252;min-height:18px;overflow-wrap:anywhere}[hidden]{display:none!important}
</style>
<h2>Hindsight</h2>
<form id="form"><label>服务地址<input id="endpoint" type="url" required placeholder="http://127.0.0.1:8888" autocomplete="off" spellcheck="false"></label><label>API Key<input id="key" type="password" autocomplete="new-password"></label><label>记忆库前缀<input id="prefix" value="mewu" required pattern="[A-Za-z0-9_-]{1,48}" autocomplete="off" spellcheck="false"></label><label class="toggle">自动同步对话<input id="sync" type="checkbox" role="switch"></label><div class="actions"><button id="test" type="button">测试连接</button><button id="save" type="button">保存</button><span id="status" role="status"></span></div></form><p id="error" role="alert"></p>
<section id="archives" hidden><h2>旧连接</h2><div id="connections"></div><button type="button" id="more" hidden>加载更多</button></section>
<script>
const $=id=>document.getElementById(id);let data,busy=true,disposed=false,keyChanged=false;
function controls(){document.querySelectorAll('input,button').forEach(e=>e.disabled=busy||e.dataset.unavailable==='true'||e.dataset.done==='true');}
function show(value){data={config:value.config,secret:value.secret};$('endpoint').value=value.config?.endpoint||'';$('prefix').value=value.config?.bankPrefix||'mewu';$('sync').checked=value.config?.completedTurnSync===true;$('key').value=value.key||'';$('key').placeholder='';keyChanged=false;}
$('key').oninput=()=>{keyChanged=true;};
async function perform(type){if(busy)return;busy=true;controls();$('error').textContent='';$('status').textContent=type==='test'?'检测中…':'';
try{const result=await mewu.call({type,config:{endpoint:$('endpoint').value,bankPrefix:$('prefix').value,completedTurnSync:$('sync').checked},...(type==='save'?{expected:data.config,secretRevision:data.secret.revision}:{}),...(keyChanged?{key:$('key').value}:{})});if(disposed)return;if(result.ok===false)throw Error(result.error);if(type==='save')show(result);$('status').textContent=type==='test'?'连接正常':'已保存';}catch(error){$('status').textContent='';$('error').textContent=error.message||'操作失败';}finally{busy=false;controls();}}
// Plugin documents cannot submit forms. Dispatch through the existing SDK,
// including Enter, without asking the host for allow-forms or network access.
function save(){if($('form').reportValidity())void perform('save');}
$('save').onclick=save;
$('form').onsubmit=e=>{e.preventDefault();save();};
$('form').onkeydown=e=>{if(e.key==='Enter'&&!e.isComposing&&e.keyCode!==229&&e.target instanceof HTMLInputElement&&e.target.type!=='checkbox'){e.preventDefault();save();}};
$('test').onclick=()=>perform('test');
let archiveCursor;
async function loadArchives(){
  const page=await mewu.call({type:'archives',cursor:archiveCursor});
  if(page.ok===false)throw Error(page.error);
  archiveCursor=page.nextCursor;$('more').hidden=!archiveCursor;
  for(const row of page.entries){
    $('archives').hidden=false;
    const line=document.createElement('div');line.className='actions';line.style.cssText='padding:12px 0;border-bottom:1px solid var(--line)';
    const text=document.createElement('span');text.style.cssText='flex:1;overflow-wrap:anywhere';text.textContent=row.agentName+' · '+row.binding.endpoint;
    const button=document.createElement('button');button.type='button';button.textContent='迁移并使用';button.dataset.unavailable=String(!row.binding.enabled||row.binding.status!=='ready');button.disabled=button.dataset.unavailable==='true';
    button.onclick=async()=>{if(busy)return;busy=true;controls();$('error').textContent='';button.textContent='迁移中…';try{
      const result=await mewu.call({type:'migrate',bindingId:row.binding.id,expectedRevision:row.binding.revision});if(result.ok===false)throw Error(result.error);button.textContent='已迁移';button.dataset.done='true';
    }catch(error){button.textContent='迁移并使用';$('error').textContent=error.message||'迁移失败';}finally{busy=false;controls();document.querySelectorAll('[data-done]').forEach(e=>e.disabled=true);}};
    line.append(text,button);$('connections').append(line);
  }
}
$('more').onclick=()=>loadArchives().catch(error=>{$('error').textContent=error.message;});
mewu.on('appearance',value=>{const root=document.documentElement,c=value?.colors||{};root.style.colorScheme=value.colorScheme||'light';if(c.text)root.style.color=c.text;for(const k of ['field','line','muted','accent'])if(c[k])root.style.setProperty('--'+k,c[k]);});
controls();mewu.ready.then(()=>mewu.call({type:'get'})).then(async value=>{if(disposed)return;show(value);await loadArchives();busy=false;controls();}).catch(e=>{$('error').textContent=e.message;busy=false;controls();});addEventListener('pagehide',()=>{disposed=true;$('key').value='';data=undefined;});
</script>`;
