// SPDX-License-Identifier: MPL-2.0
import { definePlugin } from '@mewu/sdk';

const inputSchema = {
  type: 'object',
  properties: { task: { type: 'string', maxLength: 200 } },
  additionalProperties: false
};

export default definePlugin(ctx => {
  const run = async (action, context, note) => {
    const regionId = context.regionId;
    if (!regionId) throw new Error('请在截图选区工具条上发起此命令');
    const result = await ctx.actions.invoke({ action, regionId });
    return { content: result.ok ? note : `动作未完成：${result.reason}` };
  };
  ctx.commands.register({
    id: 'example.region-actions.copy', title: '复制此选区', inputSchema,
    description: '把当前选区（含标注）复制到剪贴板。',
    metadata: { placement: 'selection-toolbar' }
  }, (input, { context }) => run('copy', context, '已复制到剪贴板'));
  ctx.commands.register({
    id: 'example.region-actions.pin', title: '贴图此选区', inputSchema,
    description: '把当前选区贴图为置顶窗口。',
    metadata: { placement: 'selection-toolbar' }
  }, (input, { context }) => run('pin', context, '已贴图'));
});
