// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createContext, runInContext } from 'node:vm';
import { stripTypeScriptTypes } from 'node:module';
const text = await readFile(new URL('./clipboard-text.ts', import.meta.url), 'utf8');
const source = stripTypeScriptTypes(text.replace(/^import .*$/m, '').replace('export async function', 'async function'), { mode: 'transform' });

test('desktop clipboard uses native publication without invoking a WebView clipboard', async () => {
  const calls = []; let finish;
  const pending = new Promise(resolve => { finish = resolve; });
  const context = createContext({ TextEncoder, isTauri: () => true, invoke: async (...args) => { calls.push(args); await pending; }, navigator: { clipboard: { writeText: () => assert.fail('desktop must use its OS channel') } } });
  runInContext(source, context);
  let committed = false; const writing = context.writeClipboardText('中文\nTEXT COPY').then(() => { committed = true; });
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [['write_clipboard_text', { text: '中文\nTEXT COPY' }]]);
  await Promise.resolve(); assert.equal(committed, false); finish(); await writing; assert.equal(committed, true);
});

test('native errors propagate and invalid payloads never reach IPC or clipboard', async () => {
  let calls = 0;
  const context = createContext({ TextEncoder, isTauri: () => true, invoke: () => { calls++; return Promise.reject(Error('clipboard locked')); } });
  runInContext(source, context);
  for (const value of ['', 'a\0b', '中'.repeat(700000)]) await assert.rejects(context.writeClipboardText(value));
  assert.equal(calls, 0); await assert.rejects(context.writeClipboardText('valid'), /clipboard locked/); assert.equal(calls, 1);
});

test('browser preview retains the ordinary browser clipboard and reports missing support', async () => {
  const values = [], context = createContext({ TextEncoder, isTauri: () => false, navigator: { clipboard: { writeText: async value => { values.push(value); } } } });
  runInContext(source, context); await context.writeClipboardText('preview'); assert.deepEqual(values, ['preview']);
  context.navigator.clipboard = undefined; await assert.rejects(context.writeClipboardText('preview'), /剪贴板不可用/);
});

test('only the first-party space receives the write-only clipboard capability', async () => {
  const root = new URL('../src-tauri/capabilities/', import.meta.url);
  for (const name of ['space', 'settings', 'scroll', 'recording-controls', 'pin', 'frozen-widget', 'extension-object']) {
    const capability = JSON.parse(await readFile(new URL(`${name}.json`, root), 'utf8'));
    assert.equal(capability.permissions.filter(value => value === 'allow-write-clipboard-text').length, name === 'space' ? 1 : 0);
  }
});
