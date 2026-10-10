// Pure preference and locale contract checks. No host, model or user storage.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';
import { createSignal } from '../../../node_modules/solid-js/dist/solid.js';

let stored = JSON.stringify({ uiLanguage: 'en-US', translationLanguage: 'ja', maskOpacity: .66, textSize: 'large' });
const document = { documentElement: { lang: '' } };
const context = createContext({ navigator: { language: 'zh-CN' }, document, Intl, localStorage: { getItem: () => stored } });
const modules = new Map();
async function load(url) {
  if (modules.has(url.href)) return modules.get(url.href);
  let module;
  if (url.pathname.endsWith('.json')) {
    const value = JSON.parse(await readFile(url, 'utf8'));
    module = new SyntheticModule(['default'], function () { this.setExport('default', value); }, { context });
  } else {
    const source = stripTypeScriptTypes(await readFile(url, 'utf8'), { mode: 'transform' });
    module = new SourceTextModule(source, { context, identifier: url.href });
  }
  modules.set(url.href, module);
  await module.link(name => name === 'solid-js'
    ? new SyntheticModule(['createSignal'], function () { this.setExport('createSignal', createSignal); }, { context })
    : load(new URL(name.endsWith('.json') ? name : `${name}.ts`, url)));
  return module;
}
const preferences = await load(new URL('./interface-preferences.ts', import.meta.url));
const locale = await load(new URL('./i18n.ts', import.meta.url));
await preferences.evaluate(); await locale.evaluate();
const { readPreferences } = preferences.namespace;
const { storedLanguage, uiLanguage, resolveLanguage, updateLanguage, t } = locale.namespace;
assert.equal(uiLanguage(), 'en-US');
updateLanguage(uiLanguage()); assert.equal(document.documentElement.lang, 'en-US');
assert.equal(t('设置'), 'Settings'); assert.equal(t('用户自定义的中文内容'), '用户自定义的中文内容');
assert.equal(readPreferences().translationLanguage, 'ja'); assert.equal(readPreferences().textSize, 'large');
updateLanguage('zh-CN'); assert.equal(t('设置'), '设置');
assert.equal(resolveLanguage('system', 'zh-TW'), 'zh-CN'); assert.equal(resolveLanguage('system', 'fr-FR'), 'en-US');
for (const invalid of ['null', '[]', 'false', '{broken']) {
  stored = invalid; assert.equal(readPreferences().maskOpacity, .58); assert.equal(storedLanguage(), 'system');
}
stored = JSON.stringify({ uiLanguage: 'invented', translationLanguage: 'invented', maskOpacity: 9, textSize: 'huge' });
assert.equal(readPreferences().maskOpacity, .8); assert.equal(readPreferences().textSize, 'default');
assert.equal(readPreferences().translationLanguage, 'zh-Hans'); assert.equal(storedLanguage(), 'system');
assert.equal(readPreferences().showButtonLabels, true); assert.equal(readPreferences().thinkingGlowEnabled, true); assert.equal(readPreferences().thinkingGlowColor, '#A7C7FF');
stored = JSON.stringify({ showButtonLabels: false, thinkingGlowEnabled: false, thinkingGlowColor: ' #f0aB91 ' });
assert.equal(readPreferences().showButtonLabels, false); assert.equal(readPreferences().thinkingGlowEnabled, false); assert.equal(readPreferences().thinkingGlowColor, '#F0AB91');
stored = JSON.stringify({ thinkingGlowColor: 'red;url(http://evil)' }); assert.equal(readPreferences().thinkingGlowColor, '#A7C7FF');
console.log('Locale preference persistence, system resolution, live switching, untouched content and invalid storage passed');
