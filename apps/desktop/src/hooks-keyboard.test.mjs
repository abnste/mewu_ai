// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
const module = new SourceTextModule(stripTypeScriptTypes(await readFile(new URL('./hooks-contracts.ts', import.meta.url), 'utf8'), {mode:'transform'}));
await module.link(() => { throw Error('Unexpected dependency'); });
await module.evaluate();
const {hookAccelerator} = module.namespace;
test('shortcut labels stay human-readable without changing stored accelerators',()=>{
  const format=module.namespace.formatHookShortcut;
  assert.equal(format('Shift+Alt+KeyS','windows'),'Shift + Alt + S');
  assert.equal(format('Super+Shift+KeyS','windows'),'Win + Shift + S');
  assert.equal(format('Super+Digit1','macos'),'⌘ + 1');
  assert.equal(format('Control+F8'),'Ctrl + F8');
});
const key = values => ({code:'',key:'',ctrlKey:false,altKey:false,shiftKey:false,metaKey:false,isComposing:false,...values});
test('remote logical keys work when no physical code is available', () => {
  assert.equal(hookAccelerator(key({key:'F8'})), 'F8');
  assert.equal(hookAccelerator(key({code:'Unidentified',key:'Y',ctrlKey:true,shiftKey:true})), 'Control+Shift+KeyY');
  assert.equal(hookAccelerator(key({key:'4',altKey:true})), 'Alt+Digit4');
});
test('physical layouts remain authoritative and ambiguous input is rejected', () => {
  assert.equal(hookAccelerator(key({code:'KeyQ',key:'a',ctrlKey:true})), 'Control+KeyQ');
  for (const values of [{key:'Dead',ctrlKey:true},{key:'?',ctrlKey:true},{key:'Unidentified',ctrlKey:true},{key:'a'},{key:'F8',isComposing:true}]) {
    assert.equal(hookAccelerator(key(values)), undefined);
  }
});
