// SPDX-License-Identifier: MPL-2.0
// Hindsight's wire protocol belongs to this independent package.
export function config(value) {
  if (!value || typeof value.endpoint !== 'string') throw Error('请配置 Hindsight 服务');
  const endpoint=value.endpoint.trim().replace(/\/+$/u,'');
  // Host URL parsing, origin-bound credentials and redirect rejection are the
  // final network boundary; this validation is for useful settings feedback.
  if(!/^https?:\/\/[^\s?#@]+$/u.test(endpoint))throw Error('服务地址无效');
  const prefix=value.bankPrefix??'mewu';
  if(typeof prefix!=='string'||!/^[A-Za-z0-9_-]{1,48}$/u.test(prefix))throw Error('记忆库前缀无效');
  return {endpoint,bankPrefix:prefix};
}
export function bank(config,agentId) {
  if(typeof agentId!=='string'||!/^[A-Za-z0-9_-]{1,128}$/u.test(agentId))throw Error('Agent 身份无效');
  return `${config.bankPrefix}-${agentId}`;
}
export function api(ctx,settings,previewKey) {
  const base=config(settings);
  if(previewKey!==undefined&&(typeof previewKey!=='string'||previewKey.length>8192||/[\x00-\x20\x7f]/u.test(previewKey)))throw Error('API Key 无效');
  return async(method,path,body,{missing=false,readiness=false}={})=>{
    const secret=previewKey===undefined?await ctx.secrets.status('api'):null;
    let response;
    try { response=await ctx.host.call('network.fetch',{
      url:base.endpoint+'/'+path.map(encodeURIComponent).join('/'),method,
      headers:{Accept:'application/json','Content-Type':'application/json',...(previewKey?{Authorization:'Bearer '+previewKey}:{})},
      ...(secret?.configured?{authorization:{key:'api'}}:{}),...(body===undefined?{}:{body})
    }); } catch { throw Error('连接中断，操作结果未知'); }
    if(response.status===404&&missing)return null;
    if(response.status===401||response.status===403)throw Error('服务拒绝认证');
    if(response.status<200||response.status>=300)throw Error(`服务返回 HTTP ${response.status}`);
    // Readiness is defined by the HTTP status, not a particular JSON body.
    if(readiness)return {ready:true};
    let result;try{result=JSON.parse(response.body);}catch{throw Error('服务返回内容无效');}
    if(!result||typeof result!=='object'||Array.isArray(result))throw Error('服务返回内容无效');
    return result;
  };
}
export const route=(id,...parts)=>['v1','default','banks',id,...parts];

export async function probe(ctx,settings,previewKey) {
  const request=api(ctx,settings,previewKey);
  const version=await request('GET',['version']);
  if(typeof version.api_version!=='string'||!/^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/u.test(version.api_version))throw Error('服务 API 不兼容');
  await request('GET',['health','ready'],undefined,{readiness:true});
  return {ok:true,version:version.api_version};
}
