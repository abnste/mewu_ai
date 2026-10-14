// SPDX-License-Identifier: MPL-2.0
import { definePlugin } from '@mewu/sdk';

const inputSchema = {
  type: 'object', properties: {
    task: { type: 'string', minLength: 1, maxLength: 16000 },
    maxSteps: { type: 'integer', minimum: 1, maximum: 30 },
    images: { type: 'array', maxItems: 8, items: { type: 'string' } },
    messages: { type: 'array', maxItems: 128, items: {
      type: 'object', properties: { role: { enum: ['user', 'assistant', 'system'] }, content: { type: 'string' } },
      required: ['role', 'content'], additionalProperties: false
    } }
  }, required: ['task'], additionalProperties: false
};

function decision(content) {
  const text = String(content).trim();
  const fenced = /^```(?:json)?\s*\n([\s\S]*)\n```$/.exec(text);
  const value = JSON.parse(fenced ? fenced[1] : text);
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Expected action object');
  if (value.done === true && typeof value.message === 'string') return value;
  if (!value.action || typeof value.action !== 'object') throw new Error('Expected action or done');
  return value;
}

export default definePlugin(ctx => {
  const run = async (input, { context }) => {
    const task = String(input.task || input.prompt || '').trim();
    if (!task || task.length > 16000) throw new Error('Task is required');
    if (!context.regionId) {
      const reply = await ctx.model.invoke({ prompt: task, ...(input.images ? { images: input.images } : {}),
        ...(input.messages ? { messages: input.messages } : {}) });
      return { content: reply.content };
    }
    const maxSteps = input.maxSteps === undefined ? 12 : input.maxSteps;
    if (!Number.isInteger(maxSteps) || maxSteps < 1 || maxSteps > 30) throw new Error('Invalid step count');
    let frame = await ctx.region.observe(context.regionId);
    const history = [];
    for (let step = 0; step < maxSteps; step++) {
      const prompt = `在用户授权的应用区域完成任务：${task}\n` +
        `当前截图宽 ${frame.width}、高 ${frame.height} 像素。坐标必须位于此图内，不是全局屏幕坐标。\n` +
        '只返回一个 JSON 对象。完成时返回 {"done":true,"message":"简短结果"}；否则返回 {"action":动作}。\n' +
        '动作只支持 {"kind":"click"或"doubleClick","x":整数,"y":整数}、' +
        '{"kind":"drag","path":[{"x":整数,"y":整数},...]}、' +
        '{"kind":"scroll","x":整数,"y":整数,"deltaY":-120到120的整数}、' +
        '{"kind":"type","text":"不含控制字符的文本"}、' +
        '{"kind":"key","key":"enter|backspace|delete|arrowLeft|arrowRight|arrowUp|arrowDown|home|end"}。\n' +
        '每次仅执行一个动作，依据接下来提供的新截图判断效果。输入框必须完整在选区内；不要操作应用之外的内容。\n' +
        `已执行：${JSON.stringify(history)}`;
      const reply = await ctx.model.invoke({ prompt, images: [frame.artifactId] });
      const next = decision(reply.content);
      if (next.done === true) return { content: next.message, steps: history.length, artifactId: frame.artifactId, completed: true };
      // Neither target coordinates nor screenshot pixels are supplied by Mewu
      // core: the independent plugin interprets this model decision itself.
      await ctx.region.action(context.regionId, next.action);
      frame = await ctx.region.observe(context.regionId);
      history.push({ step: step + 1, action: next.action, feedbackSha256: frame.sha256 });
    }
    return { content: '已达到本次操作步数', steps: history.length, artifactId: frame.artifactId, completed: false };
  };
  ctx.commands.register({
    id: 'example.region-agent.operate', title: '区域操作', inputSchema,
    metadata: { placement: 'selection-toolbar', requiresRegion: true, input: { task: '' } }
  }, run);
  ctx.agents.register({
    id: 'example.region-agent.agent', title: '区域操作 Agent', inputSchema,
    description: '观察当前区域并通过截图反馈完成操作。',
    metadata: { soul: '先观察，逐步操作，用新截图确认结果。' }
  }, run);
});
