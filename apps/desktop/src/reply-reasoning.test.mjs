// Actual disclosure state and layout observer, synthetic public text only.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, createContext, runInNewContext } from 'node:vm';
import test from 'node:test';
import { parse } from '@babel/parser';
const raw = async name => stripTypeScriptTypes(await readFile(new URL(name, import.meta.url), 'utf8'), { mode: 'transform' });
const reasoning = new SourceTextModule(await raw('./reply-reasoning.ts')); await reasoning.link(() => { throw Error('Unexpected dependency'); }); await reasoning.evaluate();
const { ReasoningDisclosure, reasoningDisplayText, splitReplyReasoning } = reasoning.namespace;

test('leading inline reasoning matches native fixtures without deleting answer code or prose', async () => {
  const cases = JSON.parse(await readFile(new URL('../tests/inline-reasoning.json', import.meta.url), 'utf8'));
  for (const entry of cases) {
    const parts = splitReplyReasoning(entry.input);
    assert.equal(parts.text, entry.text, entry.id);
    assert.equal(parts.reasoning, entry.reasoning, entry.id);
  }
  assert.equal(splitReplyReasoning('<think>same</think>answer', 'same').reasoning, 'same');
  assert.equal(splitReplyReasoning('<think>inline</think>answer', 'separate').reasoning, 'separate\n\ninline');
});

test('every streamed prefix keeps partial tags and unfinished thought out of the answer', () => {
  const envelope = '<think>SECRET🙂</think>';
  for (let length = 1; length <= envelope.length; length++) {
    assert.equal(splitReplyReasoning(envelope.slice(0, length), '', true).text, '', String(length));
  }
  assert.equal(splitReplyReasoning(envelope + '正文', '', true).text, '正文');
  assert.equal(splitReplyReasoning('<think>SECRET</thi', '', false).text, '');
  assert.equal(splitReplyReasoning('```xml\n<think>literal', '', true).text, '```xml\n<think>literal');
});

test('first live public reasoning opens, manual collapse survives deltas, and completion closes', () => {
  const view = new ReasoningDisclosure();
  assert.equal(view.update('scene/run', true, false), false);
  assert.equal(view.update('scene/run', true, true), true);
  assert.equal(view.toggle(), false);
  assert.equal(view.update('scene/run', true, true), false);
  assert.equal(view.toggle(), true);
  assert.equal(view.update('scene/run', false, true), false);
  assert.equal(view.toggle(), true);
  assert.equal(view.update('scene/run', false, true), true);
});

test('historic reasoning starts collapsed; a different scene/run resets manual state', () => {
  const view = new ReasoningDisclosure();
  assert.equal(view.update('scene/history', false, true), false);
  view.toggle(); assert.equal(view.update('other/history', false, true), false);
  assert.equal(view.update('other/new-run', true, true), true);
});

test('the view retains the entire public reasoning, including long history and surrogate pairs', () => {
  const original = 'older '.repeat(1000) + '😀'.repeat(7000);
  assert.equal(reasoningDisplayText(original), original);
  assert.equal(original.length, 20000);
  assert.equal(reasoningDisplayText('公开思考摘要'), '公开思考摘要');
});

test('async content resize follows only a reader at the tail, coalesces work and cleans up', async () => {
  const resizes = [], changes = [], tasks = [];
  class Resize { constructor(callback) { this.callback = callback; this.watched = []; resizes.push(this); } disconnect() { this.watched = []; } observe(node) { this.watched.push(node); } }
  class Mutation { constructor(callback) { this.callback = callback; changes.push(this); } observe(element, options) { this.element = element; this.options = options; } disconnect() { this.element = null; } }
  const context = createContext({ ResizeObserver: Resize, MutationObserver: Mutation, queueMicrotask: callback => tasks.push(callback) });
  const tail = new SourceTextModule(await raw('./conversation-tail.ts'), { context }); await tail.link(() => { throw Error('Unexpected dependency'); }); await tail.evaluate();
  const child = {}, host = { children: [child], scrollTop: 15, scrollHeight: 100 }, drain = () => { for (const work of tasks.splice(0)) work(); };
  let following = true;
  const stop = tail.namespace.observeConversationTail(host, () => following);
  assert.equal(resizes[0].watched.length, 2); drain(); assert.equal(host.scrollTop, 100);
  host.scrollHeight = 150; resizes[0].callback(); resizes[0].callback(); assert.equal(tasks.length, 1);
  following = false; drain(); assert.equal(host.scrollTop, 100);
  host.children.push({}); changes[0].callback(); assert.equal(resizes[0].watched.length, 3); drain(); assert.equal(host.scrollTop, 100);
  following = true; resizes[0].callback(); drain(); assert.equal(host.scrollTop, 150);
  host.scrollHeight = 200; resizes[0].callback(); stop(); drain(); assert.equal(host.scrollTop, 150);
  assert.equal(resizes[0].watched.length, 0); assert.equal(changes[0].element, null);
});

test('actual Composer clears current answer/context while retaining display history and the scene Agent identity', async () => {
  const source = await readFile(new URL('./components/Composer.tsx', import.meta.url), 'utf8'), tree = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] });
  const component = tree.program.body.find(node => node.type === 'ExportDefaultDeclaration').declaration;
  const names = ['running', 'conversationMessages', 'reply', 'latestMessage', 'replyParts', 'currentReasoning'];
  const declarations = component.body.body.filter(node => node.type === 'VariableDeclaration' && names.includes(node.declarations[0].id.name));
  assert.equal(declarations.length, names.length);
  const journal = new SourceTextModule(await raw('./run-journal.ts')); await journal.link(() => { throw Error('Unexpected dependency'); }); await journal.evaluate();
  const props = { scene: { agentId: 'one', messages: [{ id: 'old-user', role: 'user', text: '旧问题' }, { id: 'old-assistant', role: 'assistant', text: '旧回答', reasoning: '旧公开思考' }] }, stream: undefined };
  const actual = runInNewContext(declarations.map(node => source.slice(node.start, node.end)).join('\n') + ';({conversationMessages,reply,replyParts,currentReasoning})', { props, createMemo: fn => fn, latestReply: journal.namespace.latestReply, splitReplyReasoning });
  assert.equal(actual.reply().text, '旧回答'); assert.equal(actual.currentReasoning(), '旧公开思考');
  props.scene.conversationStart = props.scene.messages.length;
  assert.equal(actual.reply().text, ''); assert.equal(actual.currentReasoning(), ''); assert.equal(props.scene.messages.length, 2);
  props.stream = { runId: 'old-run', text: '过时片段', reasoning: '过时思考' };
  assert.equal(actual.reply().text, ''); assert.equal(actual.currentReasoning(), '');
  props.scene.run = { id: 'new-run', status: 'running' }; props.stream = { runId: 'new-run', text: '新片段', reasoning: '新公开思考' };
  assert.equal(actual.reply().text, '新片段'); assert.equal(actual.currentReasoning(), '新公开思考');
  props.stream = { runId: 'new-run', text: '<think>SECRET</th', reasoning: '' };
  assert.equal(actual.replyParts().text, ''); assert.equal(actual.currentReasoning(), 'SECRET');
  props.stream.text = '<think>SECRET</think>新正文';
  assert.equal(actual.replyParts().text, '新正文'); assert.equal(actual.currentReasoning(), 'SECRET');
  props.scene.messages.push({ id:'new-assistant', role:'assistant', runId:'new-run', text:'<think>历史思考</think>历史正文' });
  props.scene.run.status = 'completed';
  assert.equal(actual.replyParts().text, '历史正文'); assert.equal(actual.currentReasoning(), '历史思考');
  assert.equal(props.scene.messages.at(-1).text, '<think>历史思考</think>历史正文');
  props.scene.messages.pop();
  let disabled, title, history;
  function visit(node) {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'JSXAttribute' && node.value?.type === 'JSXExpressionContainer') {
      const value = source.slice(node.value.expression.start, node.value.expression.end);
      if (node.name.name === 'disabled' && value.includes('props.scene.messages.length') && value.includes('entry.id')) disabled = value;
      if (node.name.name === 'title' && value.includes('props.scene.messages.length') && value.includes('entry.id')) title = value;
      if (node.name.name === 'each' && value === 'props.scene.messages.map(message => message.id)') history = value;
    }
    for (const value of Object.values(node)) Array.isArray(value) ? value.forEach(visit) : visit(value);
  }
  visit(component); assert.ok(disabled); assert.ok(title); assert.ok(history);
  const choose = (entry, running = false) => ({ props, entry, running: () => running, t: text => text });
  assert.equal(actual.conversationMessages().length, 0);
  assert.equal(runInNewContext(disabled, choose({ id: 'two', name: 'Other Agent' })), true);
  assert.equal(runInNewContext(title, choose({ id: 'two', name: 'Other Agent' })), '已有对话，无法切换 Agent');
  assert.equal(runInNewContext(disabled, choose({ id: 'one', name: 'Current Agent' })), false);
  assert.equal(runInNewContext(title, choose({ id: 'one', name: 'Current Agent' })), 'Current Agent');
  assert.equal(runInNewContext(history, { props }).length, 2);
  props.scene.messages.push({ id: 'new-user', role: 'user', text: '新问题' });
  assert.equal(runInNewContext(disabled, choose({ id: 'two' })), true);
  assert.equal(runInNewContext(history, { props }).length, 3);
  props.scene.messages = []; props.scene.conversationStart = 0;
  assert.equal(runInNewContext(disabled, choose({ id: 'two', name: 'Other Agent' })), false);
  assert.equal(runInNewContext(title, choose({ id: 'two', name: 'Other Agent' })), 'Other Agent');
  assert.equal(runInNewContext(disabled, choose({ id: 'one' }, true)), true);
  assert.equal(runInNewContext(disabled, choose({ id: 'two' }, true)), true);
  assert.equal(runInNewContext(title, choose({ id: 'two' }, true)), '请求进行中');
  const english = JSON.parse(await readFile(new URL('./locales/en-US.json', import.meta.url), 'utf8'));
  assert.equal(runInNewContext(title, { ...choose({ id: 'two' }), props: { scene: { agentId: 'one', messages: [{ role: 'user' }], conversationStart: 1 } }, t: text => english[text] ?? text }), 'Conversation exists; Agent cannot be changed');
});
