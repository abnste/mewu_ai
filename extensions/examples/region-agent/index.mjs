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

const toolInputSchema = {
  type: 'object',
  properties: { task: inputSchema.properties.task, maxSteps: inputSchema.properties.maxSteps },
  required: ['task'], additionalProperties: false
};

const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const keys = (value, expected) => object(value) && Object.keys(value).length === expected.length &&
  expected.every(key => Object.prototype.hasOwnProperty.call(value, key));
const regionKeys = new Set(['enter', 'backspace', 'delete', 'arrowLeft', 'arrowRight', 'arrowUp', 'arrowDown', 'home', 'end']);
function shortInputText(text) {
  if (typeof text !== 'string' || !text.length) return false;
  let bytes = 0;
  for (const character of text) {
    const code = character.codePointAt(0);
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f) || (code >= 0xd800 && code <= 0xdfff)) return false;
    bytes += code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
    if (bytes > 4096) return false;
  }
  return true;
}
function validAction(action, frame) {
  if (!object(action)) return false;
  const point = value => keys(value, ['x', 'y']) && Number.isInteger(value.x) && Number.isInteger(value.y) &&
    value.x >= 0 && value.y >= 0 && value.x < frame.width && value.y < frame.height;
  const coordinates = () => point({ x: action.x, y: action.y });
  switch (action.kind) {
    case 'click': case 'doubleClick':
      return keys(action, ['kind', 'x', 'y']) && coordinates();
    case 'drag': {
      if (!keys(action, ['kind', 'path']) || !Array.isArray(action.path) ||
          action.path.length < 2 || action.path.length > 256 || !action.path.every(point)) return false;
      let travel = 0;
      for (let i = 1; i < action.path.length; i++) travel += Math.max(
        Math.abs(action.path[i].x - action.path[i - 1].x), Math.abs(action.path[i].y - action.path[i - 1].y));
      return travel <= 8192;
    }
    case 'scroll':
      return keys(action, ['kind', 'x', 'y', 'deltaY']) && coordinates() && Number.isInteger(action.deltaY) &&
        action.deltaY !== 0 && action.deltaY >= -1200 && action.deltaY <= 1200;
    case 'type': return keys(action, ['kind', 'text']) && shortInputText(action.text);
    case 'key': return keys(action, ['kind', 'key']) && regionKeys.has(action.key);
    default: return false;
  }
}
function decision(content, frame) {
  if (typeof content !== 'string' || content.length > 65536) return { error: 'response_text_invalid' };
  const text = content.trim();
  const fenced = /^```(?:json)?\s*\n([\s\S]*)\n```$/i.exec(text);
  let value;
  try { value = JSON.parse(fenced ? fenced[1] : text); }
  catch { return { error: 'response_json_invalid' }; }
  if (keys(value, ['done', 'message']) && value.done === true && typeof value.message === 'string' &&
      value.message.trim().length > 0 && value.message.length <= 2000) return { value };
  if (!keys(value, ['action'])) return { error: 'response_shape_invalid' };
  return validAction(value.action, frame) ? { value } : { error: 'action_invalid' };
}
function decisionPrompt(task, frame, history, correction) {
  return `在用户授权的应用区域完成任务：${task}\n` +
    `当前截图宽 ${frame.width}、高 ${frame.height} 像素。x 必须为 0 到 ${frame.width - 1} 的整数，` +
    `y 必须为 0 到 ${frame.height - 1} 的整数；不是全局屏幕坐标。\n` +
    '只返回一个完整 JSON 对象，不要解释。顶层只能是 {"action":动作对象} 或 {"done":true,"message":"简短结果"}。\n' +
    '以下每行均为格式示例，坐标必须依据截图选择，不要照抄示例坐标：\n' +
    '{"action":{"kind":"click","x":0,"y":0}}\n' +
    '{"action":{"kind":"doubleClick","x":0,"y":0}}\n' +
    '{"action":{"kind":"drag","path":[{"x":0,"y":0},{"x":0,"y":0}]}}\n' +
    '{"action":{"kind":"scroll","x":0,"y":0,"deltaY":-120}}\n' +
    '{"action":{"kind":"type","text":"示例文本"}}\n' +
    '{"action":{"kind":"key","key":"enter"}}\n' +
    '拖动路径为 2 至 256 个图内坐标，累计移动不超过 8192 像素。滚动量是 -1200 至 1200 的非零整数。\n' +
    '文本最多 4096 个 UTF-8 字节且不能含控制字符；按键只支持 enter、backspace、delete、arrowLeft、arrowRight、arrowUp、arrowDown、home、end。\n' +
    '每次只执行一个动作，根据下一张新截图确认效果。输入框必须完整在选区内；不要操作应用之外的内容。\n' +
    `已执行：${JSON.stringify(history)}` +
    (correction ? `\n上次输出未通过验证（${correction}），尚未执行该动作。请依据同一张截图重新返回严格符合上述结构的 JSON；不得添加字段或只返回内部动作。` : '');
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
      let next;
      let correction;
      // Only an unexecuted decision may be corrected. Host/model errors and
      // anything after region.action escape directly; never replay OS input.
      for (let attempt = 0; attempt < 3; attempt++) {
        const reply = await ctx.model.invoke({ prompt: decisionPrompt(task, frame, history, correction), images: [frame.artifactId] });
        const parsed = decision(reply.content, frame);
        if (parsed.value) { next = parsed.value; break; }
        correction = parsed.error;
      }
      if (!next) return { content: '模型连续返回无效操作，已停止', steps: history.length,
        artifactId: frame.artifactId, completed: false, stopReason: 'invalid_model_decision' };
      if (next.done === true) return { content: next.message, steps: history.length, artifactId: frame.artifactId, completed: true };
      // Neither target coordinates nor screenshot pixels are supplied by Mewu
      // core: the independent plugin interprets this model decision itself.
      await ctx.region.action(context.regionId, next.action);
      frame = await ctx.region.observe(context.regionId);
      history.push({ step: step + 1, action: next.action, feedbackSha256: frame.sha256 });
    }
    return { content: '已达到本次操作步数', steps: history.length, artifactId: frame.artifactId, completed: false };
  };
  ctx.tools.register({
    id: 'example.region-agent.operate', title: '区域操作', inputSchema: toolInputSchema,
    description: '观察用户明确授权的屏幕区域，根据任务逐步点击、拖动或输入，并在每次操作后截图确认结果。仅在已授权区域内执行，最多30步。',
    metadata: { requiresRegion: true }
  }, (input, invocation) => {
    if (!invocation.context.regionId) throw new Error('An explicitly authorized region is required');
    return run(input, invocation);
  });
  ctx.agents.register({
    id: 'example.region-agent.agent', title: '区域操作 Agent', inputSchema,
    description: '观察当前区域并通过截图反馈完成操作。',
    metadata: { soul: '先观察，逐步操作，用新截图确认结果。' }
  }, run);
});
