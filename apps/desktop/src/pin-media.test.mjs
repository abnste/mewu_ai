// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from 'solid-js';
import * as web from 'solid-js/web';
const raw = name => readFile(new URL(name, import.meta.url), 'utf8');
const ts = source => stripTypeScriptTypes(source, { mode: 'transform' });
const modules = new Map();
async function load(name) {
  if (modules.has(name)) return modules.get(name);
  const module = new SourceTextModule(ts(await raw(name))); modules.set(name, module);
  await module.link(dependency => load(`${dependency}.ts`)); await module.evaluate(); return module;
}
const { validPinMedia } = (await load('pin-media.ts')).namespace;
const source = 'http://127.0.0.1:49213/asset', media = { kind: 'video', sourceId: '12345678-1234-4234-9234-123456789abc', sourceDurationTicks: 100_000_000, range: { startTicks: 30_000_000, endTicks: 80_000_000 }, annotationRevision: 2, annotations: [{ id: 'annotation-1', interval: { startTicks: 40_000_000, endTicks: 50_000_000 }, bounds: { x: 10, y: 20, width: 50, height: 30 }, imageUrl: 'http://127.0.0.1:49213/overlay' }] };
test('old image pins remain compatible; timed media requires bounded local immutable descriptors', () => {
  assert.equal(validPinMedia(undefined, source), true); assert.equal(validPinMedia(media, source), true);
  for (const change of [value => value.kind = 'html', value => value.range.endTicks = 200_000_000, value => value.range.startTicks = -1, value => value.sourceDurationTicks = NaN, value => value.annotations[0].imageUrl = 'https://remote/image', value => value.annotations[0].imageUrl += '?private=1', value => value.annotations[0].interval.endTicks = 200_000_000, value => value.annotations[0].bounds.width = 0, value => value.annotations.push({ ...value.annotations[0] }), value => value.annotations = Array.from({ length: 49 }, (_, index) => ({ ...value.annotations[0], id: String(index) }))]) {
    const invalid = structuredClone(media); change(invalid); assert.equal(validPinMedia(invalid, source), false);
  }
});
test('actual pinned-video component renders real MP4 and bounded timed image layers without browser controls', async () => {
  const sourceText = await raw('components/PinVideo.tsx');
  const compiled = transformSync(sourceText, { filename: 'PinVideo.tsx', parserOpts: { plugins: ['typescript', 'jsx'] }, presets: [[solidPreset, { generate: 'ssr', hydratable: false }]], configFile: false, babelrc: false }).code;
  const synthetic = values => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); });
  const module = new SourceTextModule(ts(compiled));
  await module.link(async name => name === 'solid-js' ? synthetic(solid) : name === 'solid-js/web' ? synthetic(web) : name === 'lucide-solid' ? synthetic({ Play: () => '', Pause: () => '' }) : name.endsWith('/i18n') ? synthetic({ t: value => value }) : name.endsWith('.css') ? synthetic({}) : name.endsWith('ObjectMoveGrip') ? synthetic({ default: () => '' }) : load(`${name.replace('../', '')}.ts`));
  await module.evaluate();
  const html = web.renderToString(() => module.namespace.default({ source, media, width: 160, height: 90, onError: () => {}, onDrag: () => {} }));
  assert.ok(html.includes('<video')); assert.ok(html.includes(`src="${source}"`)); assert.ok(html.includes('playsinline')); assert.ok(html.includes('preload="auto"')); assert.equal(/<video[^>]*\bcontrols(?:=|\s|>)/.test(html), false);
  assert.ok(html.includes(`src="${media.annotations[0].imageUrl}"`)); assert.ok(html.includes('播放')); assert.equal(html.includes('下载'), false); assert.equal(html.includes('引用'), false);
});
