// node --experimental-vm-modules apps/desktop/src/pointer-bridge.test.mjs
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
const source = stripTypeScriptTypes(await readFile(new URL('./pointer-bridge.ts', import.meta.url), 'utf8'), { mode: 'transform' });
async function fixture(native) {
  const calls = [];
  const core = new SyntheticModule(['invoke', 'isTauri'], function () { this.setExport('isTauri', () => native); this.setExport('invoke', async (command, args) => { calls.push({ command, args }); return 'native-response'; }); });
  const module = new SourceTextModule(source); await module.link(specifier => { assert.equal(specifier, '@tauri-apps/api/core'); return core; }); await module.evaluate();
  return { api: module.namespace, calls };
}
const checks = [], native = await fixture(true), sourceIdentity = { width: 100, height: 100 }, request = { sceneId: 'scene', backgroundId: 'capture', x: 12, y: 34, source: sourceIdentity, path: 'ignored-private-path', sourceId: 'ignored-override' };
assert.equal(await native.api.getPointerSample(request), 'native-response');
assert.deepEqual(JSON.parse(JSON.stringify(native.calls)), [{ command: 'get_pointer_sample', args: { sceneId: 'scene', backgroundId: 'capture', x: 12, y: 34, source: sourceIdentity } }]);
checks.push('Native bridge submits only original-background identity and integer source point; no paths or override data cross IPC');
const preview = await fixture(false); assert.equal(preview.api.pointerNative, false); await assert.rejects(preview.api.getPointerSample(request), /桌面版/); assert.equal(preview.calls.length, 0);
checks.push('Ordinary browser preview explicitly rejects sampling and exposes a disabled availability flag, without fake colors');
const capabilityDir = new URL('../src-tauri/capabilities/', import.meta.url), config = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
let found = 0;
for (const file of await readdir(capabilityDir)) {
  if (!file.endsWith('.json')) continue;
  const capability = JSON.parse(await readFile(new URL(file, capabilityDir), 'utf8'));
  if (!capability.permissions.includes('allow-get-pointer-sample')) continue;
  found++; assert.deepEqual(capability.windows, ['space']); assert.ok(config.app.security.capabilities.includes(capability.identifier));
}
assert.equal(found, 1);
checks.push('The enabled native capability grants sampling only to the space window');
{
  // Tauri uses RFC7396 JSON merge: the platform windows array replaces the base
  // array. This checks actual source JSON, not a native CLI dump or focus state.
  const platform = JSON.parse(await readFile(new URL('../src-tauri/tauri.macos.conf.json', import.meta.url), 'utf8'));
  function merge(doc, patch) {
    if (!patch || typeof patch !== 'object' || Array.isArray(patch)) return structuredClone(patch);
    const result = doc && typeof doc === 'object' && !Array.isArray(doc) ? structuredClone(doc) : {};
    for (const [key, value] of Object.entries(patch)) {
      if (value === null) delete result[key];
      else result[key] = merge(result[key], value);
    }
    return result;
  }
  const capture = value => value.app.windows.find(window => window.label === 'space');
  const merged = merge(config, platform);
  assert.equal(capture(config).acceptFirstMouse, true);
  assert.equal(capture(merged).acceptFirstMouse, true);
  assert.deepEqual(merged.app.windows, platform.app.windows);
  assert.deepEqual(merged.app.security, config.app.security);
  assert.equal(capture(merged).fullscreen, false);
  assert.equal(capture(merged).visible, false);
  checks.push('Merged real macOS configuration accepts the first capture-window click and preserves platform geometry and security');
  const omitted = structuredClone(platform);
  delete capture(omitted).acceptFirstMouse;
  assert.equal(capture(merge(config, omitted)).acceptFirstMouse ?? false, false);
  checks.push('Omitting the macOS field still reproduces the false default; the base true cannot leak through a replaced windows array');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
