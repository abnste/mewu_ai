// Actual Marked/KaTeX/renderers and block reconciler; synthetic text only.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import test from 'node:test';
import { Marked } from 'marked';
import katex from 'katex';
import { decodeHTML } from 'entities';
const modules = new Map();
const deps = { marked: { Marked }, katex: { default: katex }, entities: { decodeHTML } };
async function load(name) {
  if (modules.has(name)) return modules.get(name);
  if (deps[name]) { const values = deps[name]; return new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }); }
  const code = await readFile(new URL(name + '.ts', import.meta.url), 'utf8'), module = new SourceTextModule(stripTypeScriptTypes(code, { mode: 'transform' }));
  modules.set(name, module); await module.link(load); return module;
}
const content = await load('./reply-content'); await content.evaluate();
const math = modules.get('./reply-markdown').namespace;
const { replyContent, replyWebLink, replyRasterData, createReplyBlockPatcher } = content.namespace;
const plain = value => JSON.parse(JSON.stringify(value));
const png = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQEBAQAY2E0AAAAASUVORK5CYII=';

test('HTTP(S) links preserve rich labels and only explicit safe protocols receive href', () => {
  const value = replyContent('[**文档**](https://example.com/docs?a=1&b=2 "说明")');
  assert.ok(value.html.includes('href="https://example.com/docs?a=1&amp;b=2"'));
  assert.ok(value.html.includes('<strong>文档</strong>'));
  assert.ok(value.html.includes('rel="noopener noreferrer"'));
  for (const source of ['javascript:alert(1)', 'data:text/html,hello', 'file:///C:/secret', '//example.com/a', 'https://user:password@example.com/', 'https://example.com/\nnext', 'mailto:a@example.com']) assert.equal(replyWebLink(source), undefined, source);
  for (const source of ['[危险](javascript:alert%281%29)', '[危险](javascript&#58;alert%281%29)', '[本机](file:///C:/secret)']) assert.equal(replyContent(source).html.includes('href='), false);
});

test('images have no src before any DOM parsing; external URLs and descriptions remain separate', () => {
  const value = replyContent('![示意 **图**](https://example.com/private.png)\n\n![像素](' + png + ')');
  assert.equal(value.images.length, 2);
  assert.equal(value.images[0].source, 'https://example.com/private.png');
  assert.equal(value.images[1].source, png);
  assert.ok(value.html.includes('data-mewu-image="0"'));
  assert.doesNotMatch(value.html, /<img|\bsrc=|https:\/\/|data:image\//);
  assert.equal(replyRasterData(value.images[0].source), undefined);
  assert.equal(replyRasterData(png), png);
});

test('embedded images accept bounded raster types with matching magic, never SVG/HTML/arbitrary URLs', () => {
  for (const value of ['data:image/svg+xml;base64,' + Buffer.from('<svg onload="x"/>').toString('base64'), 'data:image/png;base64,' + Buffer.from('<script>x</script>').toString('base64'), png.replace('image/png', 'image/jpeg'), png.slice(0, -1), 'blob:https://example.com/x', 'http://127.0.0.1/private', 'file:///C:/private.png']) assert.equal(replyRasterData(value), undefined, value.slice(0, 60));
  assert.equal(replyRasterData('data:image/png;base64,' + 'A'.repeat(12 * 1024 * 1024)), undefined);
  const value = replyContent('![图](https://example.com/a.png) '.repeat(20));
  assert.equal(value.images.length, 16);
  assert.equal((value.html.match(/data-mewu-image=/g) ?? []).length, 16);
});

test('formulas, code language/indentation, tables and ordered-list start survive the richer renderer', () => {
  const value = replyContent('4. 第四项\n5. 第五项\n\n```python\nfor n in values:\n    print("<&>")\n```\n\n| A | B |\n|---|---|\n| $x^2$ | \\(\\frac{1}{2}\\) |');
  assert.ok(value.html.includes('<ol start="4">'));
  assert.ok(value.html.includes('<code class="language-python">for n in values:\n    print('));
  assert.ok(value.html.includes('&lt;&amp;&gt;'));
  assert.ok(value.html.includes('<table>'));
  assert.deepEqual(Array.from(value.formulas, formula => formula.tex), ['x^2', '\\frac{1}{2}']);
  for (const formula of value.formulas) assert.match(math.renderReplyFormula(formula), /class="katex/);
});

test('raw HTML is literal and renderer overrides cannot change the trusted math parser or another call', () => {
  const value = replyContent('<script>alert(1)</script>\n\n<div onclick="x">$x$</div>');
  assert.ok(value.html.includes('&lt;script&gt;'));
  assert.equal(value.html.includes('<script>'), false);
  const raw = math.parseReplyMarkdown('![图](https://example.com/a.png)', math.escapeReplyText);
  assert.ok(raw.html.includes('<img'));
  assert.equal(replyContent('![图](https://example.com/a.png)').html.includes('<img'), false);
});

class FakeNode {
  constructor(key) { this.key = key; this.parent = null; }
  remove() { if (this.parent) { const parent = this.parent; parent.childNodes.splice(parent.childNodes.indexOf(this), 1); this.parent = null; } }
}
class FakeHost {
  constructor(nodes = []) { this.childNodes = []; for (const node of nodes) this.insertBefore(node, null); }
  insertBefore(node, anchor) { node.remove(); this.childNodes.splice(anchor ? this.childNodes.indexOf(anchor) : this.childNodes.length, 0, node); node.parent = this; }
}
test('streaming keeps stable prefix/suffix DOM nodes, replaces changed meaning, and clears stale content', () => {
  const patch = createReplyBlockPatcher(), host = new FakeHost(), signature = node => node.key;
  const first = ['paragraph A', 'live B', 'settled C'].map(value => new FakeNode(value));
  assert.equal(patch(host, new FakeHost(first), signature).length, 3);
  const grown = ['paragraph A', 'live B appended', 'settled C'].map(value => new FakeNode(value));
  const inserted = patch(host, new FakeHost(grown), signature);
  assert.equal(inserted.length, 1); assert.equal(host.childNodes[0], first[0]); assert.equal(host.childNodes[2], first[2]);
  assert.equal(first[1].parent, null);
  // A changed reference definition has a changed final href/key, even if its raw block text stayed equal.
  patch(host, new FakeHost([new FakeNode('changed reference href'), new FakeNode('live B appended'), new FakeNode('settled C')]), signature);
  assert.notEqual(host.childNodes[0], first[0]); assert.equal(host.childNodes[2], first[2]);
  const retained = [...host.childNodes]; assert.equal(patch(host, new FakeHost(retained.map(node => new FakeNode(node.key))), signature).length, 0);
  assert.deepEqual(host.childNodes, retained);
  patch(host, new FakeHost(), signature); assert.equal(host.childNodes.length, 0);
});

test('native links are narrow and validated; browser links do not dispatch IPC', async () => {
  let native = false; const calls = [];
  const code = stripTypeScriptTypes(await readFile(new URL('./reply-content-bridge.ts', import.meta.url), 'utf8'), { mode: 'transform' });
  const bridge = new SourceTextModule(code);
  await bridge.link(name => name === './reply-content' ? content : new SyntheticModule(['invoke', 'isTauri'], function () { this.setExport('isTauri', () => native); this.setExport('invoke', async (...args) => calls.push(args)); }));
  await bridge.evaluate();
  await bridge.namespace.openReplyLink('https://example.com/docs'); assert.equal(calls.length, 0);
  native = true; await bridge.namespace.openReplyLink('https://example.com/docs');
  assert.deepEqual(plain(calls), [['open_web_link', { url: 'https://example.com/docs' }]]);
  await assert.rejects(bridge.namespace.openReplyLink('javascript:alert(1)')); assert.equal(calls.length, 1);
});
