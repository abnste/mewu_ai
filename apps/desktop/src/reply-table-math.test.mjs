// Actual Marked/KaTeX and Solid SSR. No clipboard, model, files or network.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { test } from 'node:test';
import { transformSync } from '@babel/core';
import solidPreset from 'babel-preset-solid';
import * as solid from 'solid-js';
import * as web from 'solid-js/web';
import { Marked } from 'marked';
import katex from 'katex';
import { decodeHTML } from 'entities';

const modules = new Map(), sanitized = [];
const external = {
  marked: { Marked }, katex: { default: katex }, entities: { decodeHTML }, 'solid-js': solid, 'solid-js/web': web,
  '@tauri-apps/api/core': { isTauri: () => false, invoke: () => { throw Error('Unexpected native IPC'); } },
  '../i18n': { t: value => value },
  // SSR has no browser DOM. Assert the sanitizer boundary sees generated KaTeX
  // only; actual DOMPurify + Range copy are checked by the browser fixture.
  dompurify: { default: { sanitize(html, options) { assert.match(html, /^<span class="katex/); assert.equal(options.ALLOW_DATA_ATTR, false); sanitized.push(html); return html; } } },
  'lucide-solid': Object.fromEntries(['Check', 'ChevronDown', 'Copy', 'LoaderCircle'].map(name => [name, () => ''])),
};
async function load(name, parent = import.meta.url) {
  if (name.endsWith('.css')) return new SyntheticModule([], () => {});
  if (external[name]) { const values = external[name]; return new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }); }
  if (name.endsWith('/table-bridge')) return new SyntheticModule(['exportMessageTable'], function () { this.setExport('exportMessageTable', () => { throw Error('Unexpected export'); }); });
  const url = new URL(name.startsWith('.') ? name : './' + name, parent);
  if (!/\.tsx?$/.test(url.pathname)) url.pathname += '.ts';
  if (modules.has(url.href)) return modules.get(url.href);
  const raw = await readFile(url, 'utf8');
  const source = url.pathname.endsWith('.tsx')
    ? stripTypeScriptTypes(transformSync(raw, { filename: url.pathname, presets: [[solidPreset, { generate: 'ssr', hydratable: false }]], parserOpts: { plugins: ['typescript', 'jsx'] }, configFile: false, babelrc: false }).code, { mode: 'transform' })
    : stripTypeScriptTypes(raw, { mode: 'transform' });
  const module = new SourceTextModule(source, { identifier: url.href }); modules.set(url.href, module);
  await module.link(dependency => load(dependency === './Markdown' ? './Markdown.tsx' : dependency, url.href));
  return module;
}
const helper = await load('./reply-table-math.ts'); await helper.evaluate();
const preview = modules.get(new URL('./table-preview.ts', import.meta.url).href).namespace;
const math = modules.get(new URL('./reply-markdown.ts', import.meta.url).href).namespace;
const { replyTablePresentation } = helper.namespace;
const table = text => preview.previewMessageTables(text, 'synthetic').tables[0];
const formulas = value => value?.flat(2).filter(part => part.formula).map(part => part.formula) ?? [];
const plain = value => value.map(row => row.map(cell => cell.map(part => part.text).join('')));

test('Header/body fractions, roots, matrix and Chinese math render; host cells and all text exports are unchanged', () => {
  const source = String.raw`| $x^2$ | 答案 |
|:---|---:|
| $\frac{1}{2}$ | \(\sqrt{2}\) |
| $$\begin{pmatrix}a&b\\c&d\end{pmatrix}$$ | $\text{中文}=2$ |`;
  const native = table(source), before = JSON.stringify(native);
  const outputs = ['markdown','csv','tsv'].map(kind => preview.tablePlainText(native, kind));
  const value = replyTablePresentation(source, native);
  assert.deepEqual(plain(value), [native.header, ...native.rows]);
  assert.equal(formulas(value).length, 5);
  for (const formula of formulas(value)) assert.match(math.renderReplyFormula(formula), /class="katex/);
  assert.equal(JSON.stringify(native), before);
  assert.deepEqual(['markdown','csv','tsv'].map(kind => preview.tablePlainText(native, kind)), outputs);
});

test('Code, escaped dollars, currency, image alt and raw code are data; HTML cannot become cell markup', () => {
  const source = String.raw`| 左 | 右 |
|---|---|
| ${'`$x$`'} | <code>$y$</code> |
| \$z$ | $5 and $10 |
| ![$q$](https://invalid.test/image) | <img src=x onerror=alert(1)> $a$ |
| [**$b$**](https://invalid.test) | &lt;script&gt; |`;
  const native = table(source), value = replyTablePresentation(source, native);
  assert.deepEqual(plain(value), [native.header, ...native.rows]);
  assert.deepEqual(formulas(value).map(value => value.tex), ['a', 'b']);
  assert.equal(native.rows[2][1], ' $a$');
  assert.equal(native.rows[3][1], '<script>');
});

test('Native identity is complete cells/alignment, never a guessed index; ambiguous duplicates stay literal', () => {
  const raw = '| A |\n|---|\n| `$x$` |', mathSource = '| A |\n|---|\n| $x$ |';
  const native = table(mathSource);
  assert.equal(formulas(replyTablePresentation(mathSource, { ...native, index: 999 })).length, 1);
  assert.equal(replyTablePresentation(raw + '\n\n' + mathSource, native), undefined);
  assert.equal(formulas(replyTablePresentation(mathSource + '\n\n' + mathSource, native)).length, 1);
  for (const altered of [{ ...native, header: ['B'] }, { ...native, rows: [['$y$']] }, { ...native, align: ['right'] }, { ...native, rows: [...native.rows, ['more']] }]) assert.equal(replyTablePresentation(mathSource, altered), undefined);
  assert.equal(replyTablePresentation('```md\n' + mathSource + '\n```', native), undefined);
});

test('Nested tables and escaped pipes retain row/column shape and parser disagreement falls back exactly', () => {
  const source = String.raw`> | A | B |
> |---|---|
> | $x\vert y$ | ${'`a\\|b`'}<br>后文 |

- 表格

  | C | D |
  |---|---|
  | $z$ | **bold** |`;
  const tables = preview.previewMessageTables(source, 'synthetic').tables;
  assert.equal(tables.length, 2);
  for (const native of tables) assert.deepEqual(plain(replyTablePresentation(source, native)), [native.header, ...native.rows]);
  assert.equal(tables[0].rows[0][1], 'a|b\n后文');
  const native = { ...tables[0], rows: [['$x\\vert y$', 'a|b 后文']] };
  assert.equal(replyTablePresentation(source, native), undefined);
});

test('Formula budget is shared across source tables; invalid, partial and over-limit formulas remain copyable', () => {
  const source = '| A |\n|---|\n' + Array.from({ length: 129 }, () => '| $x$ |').join('\n');
  const native = table(source), value = replyTablePresentation(source, native);
  assert.equal(formulas(value).length, 128); assert.equal(value[129][0][0].text, '$x$');
  for (const expression of ['$\\unknown{a}$', '$\\def\\a{\\a}\\a$', '$' + 'x'.repeat(2050) + '$', '\\(unclosed']) {
    const text = `| A |\n|---|\n| ${expression} |`, document = table(text), display = replyTablePresentation(text, document);
    assert.deepEqual(plain(display), [document.header, ...document.rows]);
    for (const formula of formulas(display)) assert.equal(math.renderReplyFormula(formula), undefined);
  }
  assert.equal(replyTablePresentation(' '.repeat(1024 * 1024 + 1), native), undefined);
  assert.equal(replyTablePresentation(Array.from({length: 13}, () => '| A |\n|---|\n| $x$ |').join('\n\n'), table('| A |\n|---|\n| $x$ |')), undefined);
});

test('Actual Solid cell rendering uses generated KaTeX only, escapes literal HTML and preserves fallback/copy spelling', async () => {
  const component = await load('./components/ReplyTable.tsx'); await component.evaluate();
  const { ReplyTableCell } = component.namespace;
  const source = String.raw`| A | B |
|---|---|
| $\frac{1}{2}$ | &lt;img src=x onerror=alert(1)&gt; |
| $\unknown{a}$ | ${'`$x$`'} |`;
  const native = table(source), value = replyTablePresentation(source, native);
  const render = (r, c) => web.renderToString(() => solid.createComponent(ReplyTableCell, { text: [native.header, ...native.rows][r][c], parts: value[r][c] }));
  assert.match(render(1, 0), /class="katex/); assert.match(render(1, 0), /data-formula-source="\$\\frac\{1\}\{2\}\$"/);
  assert.doesNotMatch(render(1, 1), /<img|onerror="/); assert.match(render(1, 1), /&lt;img/);
  assert.match(render(2, 0), /\$\\unknown\{a\}\$/); assert.doesNotMatch(render(2, 0), /class="katex/);
  assert.match(render(2, 1), /\$x\$/); assert.doesNotMatch(render(2, 1), /class="katex/);
  assert.ok(sanitized.length >= 1);
});
