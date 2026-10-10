// node --experimental-vm-modules apps/desktop/src/reply-markdown.test.mjs
// Actual Marked/KaTeX, synthetic reply strings; no network or clipboard.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';
import { Marked } from 'marked';
import katex from 'katex';
const source = await readFile(new URL('./reply-markdown.ts', import.meta.url), 'utf8');
const module = new SourceTextModule(stripTypeScriptTypes(source, { mode: 'transform' }));
await module.link(async name => {
  const exports = name === 'marked' ? { Marked } : name === 'katex' ? { default: katex } : null;
  assert.ok(exports, `Unexpected dependency ${name}`);
  return new SyntheticModule(Object.keys(exports), function () { for (const [key, value] of Object.entries(exports)) this.setExport(key, value); });
});
await module.evaluate();
const { parseReplyMarkdown, renderReplyFormula, escapeReplyText, MAX_FORMULA_LENGTH } = module.namespace;
const parse = text => parseReplyMarkdown(text, escapeReplyText);
const checks = [];
{
  const value = parse('面积 $a^2$。\n\n$$\\frac{1}{2}$$\n\n\\(x+y\\) 与 \\[\\sqrt{2}\\]');
  assert.deepEqual(Array.from(value.formulas, value => [value.tex, value.display]), [['a^2', false], ['\\frac{1}{2}', true], ['x+y', false], ['\\sqrt{2}', true]]);
  for (const formula of value.formulas) assert.match(renderReplyFormula(formula), /class="katex/);
  checks.push('Four delimiters render actual KaTeX with inline/display identities');
}
{
  const value = parse('```tex\n$x$\n\\(y\\)\n```\n\n`$z$` 与 \\$10 和 $a$');
  assert.deepEqual(Array.from(value.formulas, v => v.tex), ['a']);
  assert.match(value.html, /<pre><code class="language-tex">/);
  assert.match(value.html, /<code>\$z\$<\/code>/);
  assert.equal(parse('<code>$x$</code> <pre>\\(y\\)</pre>').formulas.length, 0);
  checks.push('Fenced code, code spans, raw code tags and escaped dollars remain literal');
}
{
  for (const text of ['$5 and $10', '价格 $5，另 $10', '$ 20 $', 'USD $100.00', 'Cost $5 / $10 each', '$x\ny$']) assert.equal(parse(text).formulas.length, 0, text);
  assert.equal(parse('方程 $2x+1=5$。').formulas.length, 1);
  assert.equal(parse('$5+2$').formulas.length, 1);
  const mixed = parse('价格 $5 和 $10；行内代码 `$x^2$`。');
  assert.equal(mixed.formulas.length, 0);
  assert.match(mixed.html, /<code>\$x\^2\$<\/code>/);
  for (const text of ['价格 $5，[公式 $x$](https://example.com)', '价格 $5，<code>$x$</code>']) {
    const value = parse(text); assert.ok(!value.formulas.some(formula => formula.tex.startsWith('5')));
  }
  checks.push('Currency and unclosed/newline dollar spans do not become formulas');
}
{
  const full = '结论 \\(x+1\\)'.replaceAll('\\\\', '\\');
  for (let size = 0; size <= full.length; size++) {
    const value = parse(full.slice(0, size));
    assert.ok(value.formulas.length <= 1);
    if (size < full.length) assert.equal(value.formulas.length, 0);
  }
  assert.match(parse('待补 \\(\\frac{1}').html, /\\\(/);
  assert.equal(parse('$' + 'a'.repeat(MAX_FORMULA_LENGTH + 2) + '$').formulas.length, 0);
  assert.equal(parse('$x$ '.repeat(200)).formulas.length, 128);
  checks.push('Streaming partial brackets stay literal; per-formula and per-reply bounds hold');
}
{
  const raw = '<span data-mewu-formula="0" style="color:red"><img src=x onerror=alert(1)></span>';
  const value = parse(raw + '\n\n$x$');
  assert.equal(value.formulas.length, 1);
  assert.ok(value.html.includes('&lt;span'));
  assert.equal((value.html.match(/<span data-mewu-formula=/g) ?? []).length, 1);
  for (const tex of ['\\htmlStyle{background:url(https://evil)}{x}', '\\htmlData{evil=1}{x}', '\\includegraphics{https://evil/a.png}', '\\href{javascript:alert(1)}{x}']) {
    const rendered = renderReplyFormula({ tex, source: '$' + tex + '$', display: false });
    assert.ok(!rendered || !/<(?:img|script)|href=|background:url|data-evil=/.test(rendered), tex);
  }
  assert.equal(renderReplyFormula({ tex: '\\def\\x{\\x}\\x', source: '', display: false }), undefined);
  assert.equal(renderReplyFormula({ tex: '\\gdef\\custom{X}\\custom', source: '', display: false })?.includes('X'), true);
  assert.equal(renderReplyFormula({ tex: '\\custom', source: '', display: false }), undefined);
  assert.match(renderReplyFormula({ tex: '\\text{中文}=2', source: '', display: false }), /中文/);
  checks.push('Untrusted HTML stays in raw sanitizer, dangerous TeX cannot add DOM/URLs, macros are bounded and isolated');
}
{
  const plainParser = new Marked();
  parse('$$x$$');
  assert.ok(!plainParser.parse('$x$').includes('data-mewu-formula'));
  const table = parse('| A | B |\n|---|---|\n| $x$ | `code` |');
  assert.match(table.html, /<table>/); assert.match(table.html, /<code>code<\/code>/);
  const component = await readFile(new URL('./components/Markdown.tsx', import.meta.url), 'utf8');
  assert.match(component, /closest\('pre,code'\)/);
  assert.doesNotMatch(component, /closest\('pre,code,table'\)/);
  assert.match(component, /DOMPurify\.sanitize\(rendered/);
  assert.match(component, /data\.formulaSource|dataset\.formulaSource/);
  checks.push('Parser is instance-local; streaming table math uses typed tokens and KaTeX output has a separate sanitizer');
}
{
  // Actual 0.7.3 geometry/vision regression strings, rendered by actual KaTeX.
  const formulas = [
    String.raw`AD\perp CE`, String.raw`\mathbf a=\overrightarrow{AB}`, String.raw`\boxed{\frac1{15}}`,
    String.raw`\mathbf a\cdot\mathbf c=\frac{8^{2}+3^{2}-7^{2}}2=12,\qquad \mathbf b\cdot\mathbf c=\frac{5^{2}+3^{2}-5^{2}}2=\frac92.`,
    String.raw`\overrightarrow{CE}=\frac38\mathbf a-\mathbf b`,
    String.raw`\overrightarrow{AD}\cdot\overrightarrow{CE}=\frac38\times12-\frac92=0,`,
    String.raw`|\mathbf u|=\sqrt{25-\frac{20^{2}}{64}}=\frac{5\sqrt3}{2},\qquad |\mathbf v|=\sqrt{9-\frac{12^{2}}{64}}=\frac{3\sqrt3}{2}.`,
    String.raw`\cos\angle(C-AB-D)=\frac{\frac34}{\sqrt{\frac{75}{4}}\sqrt{\frac{27}{4}}}=\frac1{15}.`,
    String.raw`a\in[\frac1e,+\infty)`, String.raw`x\to1^+`, String.raw`a(\ln a+1)\ge0`,
    String.raw`\frac{1}{\sqrt{\pi}}\int\_{-\infty}^{x}\frac{1}{2\sqrt{t-\tau}}\\,e^{-\frac{(x+\xi)^2}{4(t-\tau)}}\\,d\xi`,
    String.raw`(x+a)^{n}=\sum\_{k=0}^{n}\binom{n}{k}x^{k}a^{n-k}`,
    String.raw`\begin{aligned}x+1&=2\\x&=1\end{aligned}`,
  ];
  for (const tex of formulas) {
    const value = parse(`\\[${tex}\\]`).formulas[0];
    assert.equal(value.tex, tex); assert.equal(value.source, `\\[${tex}\\]`);
    assert.match(renderReplyFormula(value), /class="katex/);
    const expected = katex.renderToString(tex.replaceAll('\\_', '_').replaceAll('\\\\,', '\\,'), { displayMode: true, output: 'htmlAndMathml', trust: false, throwOnError: true, strict: 'error', maxExpand: 200, maxSize: 20, macros: {}, globalGroup: false });
    assert.equal(renderReplyFormula(value), expected);
  }
  assert.equal(parse('```tex\n' + formulas[11] + '\n```').formulas.length, 0);
  checks.push('Original 0.7.3 geometry, vision, shorthand fractions, binomial and aligned formulas keep source and render actual equivalent TeX');
}
{
  const source = String.raw`(1) 切线方程：$y=\frac{x-1}{e}$。\n(2) 参数范围：$a\in[\frac1e,+\infty)$。必要性由$x\to1^+$得$a(\ln a+1)\ge0$。`;
  const value = parse(source);
  assert.equal(value.formulas.length, 4); assert.match(value.html, /<br>\(2\)/);
  for (const formula of value.formulas) assert.match(renderReplyFormula(formula), /class="katex/);
  for (let end = 0; end <= source.length; end += 5) assert.doesNotThrow(() => parse(source.slice(0, end)));
  assert.match(parse('`' + source + '`').html, /\\n\(2\)/);
  assert.match(parse('```text\n' + source + '\n```').html, /\\n\(2\)/);
  assert.match(parse(String.raw`<code>\n(2)</code>`).html, /\\n\(2\)/);
  assert.match(parse(String.raw`这是 \nu 和 \nabla。`).html, /\\nu.*\\nabla/);
  assert.match(parse(String.raw`\\n(2) 保留转义斜线`).html, /\\n\(2\)/);
  checks.push('Original provider escaped item break is adapted inside Marked only; code, escaped slashes and LaTeX commands stay literal');
}
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
