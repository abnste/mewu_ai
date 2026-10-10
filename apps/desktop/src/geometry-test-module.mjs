// Shared VM loader for the real geometry preview model; no DOM/native dependencies.
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
export async function loadProviderPresets(context) {
  const source = stripTypeScriptTypes(await readFile(new URL('./provider-presets.ts', import.meta.url), 'utf8'), { mode: 'transform' });
  const module = new SourceTextModule(source, context ? { context } : undefined);
  await module.link(() => { throw Error('Unexpected provider preset runtime import'); });
  await module.evaluate(); return module;
}
export async function loadAssetImport(context) {
  if (context && !context.TextDecoder) context.TextDecoder = TextDecoder;
  const source = stripTypeScriptTypes(await readFile(new URL('./asset-import.ts', import.meta.url), 'utf8'), { mode: 'transform' });
  const module = new SourceTextModule(source, context ? { context } : undefined);
  await module.link(() => { throw Error('Unexpected asset import'); });
  await module.evaluate(); return module;
}
export async function loadGeometryHistory(context) {
  if (context && !context.TextEncoder) context.TextEncoder = TextEncoder;
  const read = async name => stripTypeScriptTypes(await readFile(new URL(name, import.meta.url), 'utf8'), { mode: 'transform' });
  const geometry = new SourceTextModule(await read('./region-geometry.ts'), context ? { context } : undefined);
  await geometry.link(() => { throw Error('Unexpected geometry import'); });
  const history = new SourceTextModule(await read('./region-geometry-history.ts'), context ? { context } : undefined);
  await history.link(name => { if (name === './region-geometry') return geometry; throw Error(`Unexpected history import ${name}`); });
  await history.evaluate(); return history;
}
