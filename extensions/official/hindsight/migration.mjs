// SPDX-License-Identifier: MPL-2.0
// This plugin owns the Hindsight route and migration decisions. The host only
// exposes a bounded archive and copies encrypted credentials without disclosure.
export async function archives(ctx,cursor) {
  const page=await ctx.host.call('memory.archive.list',{...(cursor?{cursor}:{})});
  return {...page,entries:page.entries.filter(row=>row.binding.providerId==='hindsight')};
}
export async function migrate(ctx,input) {
  let cursor,chosen;
  do {
    const page=await archives(ctx,cursor);
    chosen=page.entries.find(row=>row.binding.id===input.bindingId);
    cursor=page.nextCursor;
  }while(!chosen&&cursor);
  if(!chosen||chosen.binding.revision!==input.expectedRevision)throw Error('旧连接已变化，请重新载入');
  const {binding:b}=chosen;
  if(!b.enabled||b.status!=='ready')throw Error('请先确认旧连接状态');
  const key='archive.'+b.agentId,previous=await ctx.configuration.get(key);
  if(previous&&previous.bindingId!==b.id)throw Error('该 Agent 已迁移其他连接');
  const secretKey='archive.'+b.id,secret=await ctx.secrets.status(secretKey);
  await ctx.host.call('memory.archive.credential',{bindingId:b.id,expectedRevision:b.revision,secretRevision:secret.revision});
  const next={bindingId:b.id,endpoint:b.endpoint,bankId:chosen.bankId,policy:b.policy};
  if(!await ctx.configuration.compareAndSet(key,previous,next))throw Error('配置已变化，请重新载入');
  // Publish the executable binding last. A stopped migration leaves the old
  // archive intact and can resume; it never dispatches uncertain old work.
  await ctx.host.call('memory.archive.activate',{bindingId:b.id,expectedRevision:b.revision,bindingRevision:chosen.bindingRevision,serviceId:'memory'});
  return {ok:true};
}
export async function legacyDocuments(ctx,archive,ids) {
  if(!archive||!ids.length)return [];
  return await ctx.host.call('memory.archive.lookup',{bindingId:archive.bindingId,documentIds:ids.slice(0,128)});
}
