// Actual Solid Composer SSR + pure presentation policy. No browser, IPC or user data.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import test from 'node:test';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from 'solid-js';
import * as web from 'solid-js/web';
import postcss from 'postcss';

const loadPlain = async name => {
  const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(name, import.meta.url), 'utf8'), { mode: 'transform' }));
  await module.link(() => { throw Error('Unexpected dependency'); }); await module.evaluate(); return module.namespace;
};
const policy = await loadPlain('./conversation-presentation.ts'), reasoning = await loadPlain('./reply-reasoning.ts'), journal = await loadPlain('./run-journal.ts');
const user = (id, runId, text = id) => ({ id, runId, role: 'user', text });
const assistant = (id, runId, text = id) => ({ id, runId, role: 'assistant', text });

test('compact prompt belongs to the exact current run and context slice, including terminal runs', () => {
  const messages = [user('old-user', 'old'), assistant('old-answer', 'old'), user('new-user', 'new'), assistant('new-answer', 'new')];
  assert.equal(policy.currentTurnPrompt(messages, 'new'), messages[2]);
  assert.equal(policy.currentTurnPrompt(messages, 'missing'), undefined);
  assert.equal(policy.currentTurnPrompt(messages), messages[2]);
  assert.equal(policy.currentTurnPrompt(messages.slice(4), 'new'), undefined);
  assert.equal(policy.currentTurnPrompt([user('legacy', undefined)], 'current'), undefined);
  assert.equal(messages.length, 4);
});

test('thinking glow disappears on completion, cancellation, freeze, close and preference disable', () => {
  const scene = { frozen: false, closed: false, run: { status: 'running' } };
  assert.equal(policy.thinkingGlowVisible(scene, false), true);
  for (const status of ['completed', 'failed', 'canceled']) assert.equal(policy.thinkingGlowVisible({ ...scene, run: { status } }, false), false);
  for (const value of [{ ...scene, closed: true }, { ...scene, frozen: true }, { ...scene, run: undefined }]) assert.equal(policy.thinkingGlowVisible(value, false), false);
  assert.equal(policy.thinkingGlowVisible(scene, true), false); assert.equal(policy.thinkingGlowVisible(scene, false, false), false);
  assert.equal(policy.thinkingGlowColor('#abcdef'), '#ABCDEF');
  for (const value of [undefined, 'red', '#123', '#A7C7FF;background:url(secret)', 'var(--secret)', '<img>']) assert.equal(policy.thinkingGlowColor(value), '#A7C7FF');
});

const escaped = value => String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const plainAnswer = props => web.ssr(`<div class="message-markdown">${escaped(props.text)}</div>`);
const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
const source = await readFile(new URL('./components/Composer.tsx', import.meta.url), 'utf8');
const transformed = transformSync(source, { filename: 'Composer.tsx', presets: [[solidPreset, { generate: 'ssr', hydratable: false }]], parserOpts: { plugins: ['typescript', 'jsx'] }, configFile: false, babelrc: false }).code;
const composerModule = new SourceTextModule(stripTypeScriptTypes(transformed, { mode: 'transform' }));
const icons = Object.fromEntries(['Bot', 'Check', 'ChevronDown', 'ChevronUp', 'CircleAlert', 'Cpu', 'FolderOpen', 'History', 'Link2', 'LoaderCircle', 'MessageSquare', 'Minus', 'RotateCcw', 'Send', 'Square', 'SquarePen', 'X'].map(name => [name, () => '']));
await composerModule.link(name => {
  if (name.endsWith('.css')) return synthetic({});
  if (name === 'solid-js') return synthetic(solid);
  if (name === 'solid-js/web') return synthetic(web);
  if (name === 'lucide-solid') return synthetic(icons);
  if (name.endsWith('/conversation-presentation')) return synthetic(policy);
  if (name.endsWith('/reply-reasoning')) return synthetic(reasoning);
  if (name.endsWith('/run-journal')) return synthetic(journal);
  if (name.endsWith('/conversation-tail')) return synthetic({ observeConversationTail: () => () => {} });
  if (name.endsWith('/i18n')) return synthetic({ t: value => value });
  if (name === './Markdown' || name === './TableAnswer') return synthetic({ default: plainAnswer });
  if (['./ToolProgress', './RunJournal', './ReasoningView', './VideoAnswerActions'].includes(name)) return synthetic({ default: () => '' });
  if (name === './VoiceInputButton') return synthetic({ default: () => web.ssr('<div class="voice-input-control"><button class="composer-round voice-trigger"></button><button class="voice-language-trigger"></button></div>') });
  throw Error(`Unexpected dependency ${name}`);
});
await composerModule.evaluate();
const noop = () => {};
const props = extra => ({ scene: { id: 'scene', agentId: 'agent', regions: [], items: [], messages: [], frozen: false, closed: false }, agents: [], connections: [], draft: '', refs: [], expanded: false, sending: false, closing: false, selectionActive: false, focusRequest: 0, onPosition: noop, onDraft: noop, onSend: noop, onCancel: noop, onImport: noop, onFreeze: noop, onNew: noop, onCapture: noop, onClose: noop, onSessions: noop, onRemoveRef: noop, onFocusRef: noop, onExpanded: noop, onAgent: noop, onConnection: noop, onSelectConnection: noop, onContinueJournal: noop, ...extra });
const render = async extra => {
  const previousDocument = globalThis.document, previousWindow = globalThis.window;
  globalThis.document = { addEventListener: noop, removeEventListener: noop };
  globalThis.window = { addEventListener: noop, removeEventListener: noop };
  try { const html = web.renderToString(() => composerModule.namespace.default(props(extra))); await new Promise(resolve => setTimeout(resolve, 10)); return html; }
  finally { globalThis.document = previousDocument; globalThis.window = previousWindow; }
};

test('actual compact Composer keeps prompt and answer as separate bubbles while old turns remain expanded history', async () => {
  const scene = { ...props().scene, messages: [user('old-user', 'old', '旧问题'), assistant('old-answer', 'old', '旧回答'), user('new-user', 'new', '<script>新问题</script>'), assistant('new-answer', 'new', '新回答')], run: { id: 'new', status: 'completed' } };
  const compact = await render({ scene });
  assert.equal((compact.match(/<article\b/g) ?? []).length, 2);
  assert.match(compact, /class="message user"/); assert.match(compact, /class="message assistant"/);
  assert.match(compact, /&lt;script(?:>|&gt;)新问题&lt;\/script(?:>|&gt;)/); assert.doesNotMatch(compact, /旧问题|旧回答|<script>/);
  assert.equal(((await render({ scene, expanded: true })).match(/<article\b/g) ?? []).length, 4);
  assert.doesNotMatch(await render({ scene: { ...scene, conversationStart: 4, run: undefined } }), /新问题|新回答|旧问题|旧回答/);
  const interrupted = await render({ scene: { ...scene, messages: scene.messages.slice(0, 3), run: { id: 'new', status: 'failed' } } });
  assert.match(interrupted, /新问题/); assert.doesNotMatch(interrupted, /旧回答/);
});

test('actual running Composer displays just one live response and a passive bottom glow with unchanged toolbar controls', async () => {
  const scene = { ...props().scene, messages: [user('user', 'run', '当前问题')], run: { id: 'run', status: 'running' } };
  const compact = await render({ scene, stream: { runId: 'run', text: '流式回答' }, voice: {} });
  assert.equal((compact.match(/流式回答/g) ?? []).length, 1);
  assert.match(compact, /space-thinking-glow/); assert.match(compact, /aria-hidden="true"/);
  assert.equal((compact.match(/composer-button-label/g) ?? []).length, 4);
  assert.match(compact, /composer-button-label">停止/);
  const disabled = await render({ scene, showButtonLabels: false, thinkingGlowEnabled: false, voice: {} });
  assert.doesNotMatch(disabled, /space-thinking-glow|composer-button-label/);
  assert.equal((disabled.match(/<button\b/g) ?? []).length, (compact.match(/<button\b/g) ?? []).length);
  assert.match(disabled, /aria-label="停止"/);
  assert.doesNotMatch(await render({ scene: { ...scene, frozen: true } }), /space-thinking-glow/);
});

test('conversation CSS preserves original two-sided margins, bounded non-hit-test glow and shared voice sizes', async () => {
  const sheet = postcss.parse(await readFile(new URL('./conversation.css', import.meta.url), 'utf8'));
  const declarations = selector => { const found = []; sheet.walkRules(rule => { if (rule.selector === selector) { const values = {}; rule.walkDecls(decl => values[decl.prop] = decl.value); found.push(values); } }); return found; };
  const bubble = declarations('.app .mewu-composer .message')[0];
  assert.equal(bubble['max-width'], 'calc(100% - 44px)'); assert.equal(bubble.border, '1px solid #dce4f1'); assert.equal(bubble.width, 'fit-content');
  assert.equal(declarations('.app .mewu-composer .message.user')[0]['margin-left'], 'auto');
  assert.equal(declarations('.app .mewu-composer .message.assistant')[0]['margin-right'], 'auto');
  const glow = declarations('.space-thinking-glow')[0];
  assert.equal(glow['pointer-events'], 'none'); assert.equal(glow.bottom, '0'); assert.equal(glow.height, 'min(220px,24vh)');
  assert.equal(declarations('.reduce-motion .space-thinking-glow')[0].animation, 'none');
  sheet.walkRules(rule => assert.equal(rule.selector.includes('voice-trigger') || rule.selector.includes('composer-round'), false, 'Captions must not override shared 38px/34px buttons'));
});
