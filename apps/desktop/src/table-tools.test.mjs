// node --experimental-vm-modules apps/desktop/src/table-tools.test.mjs
// Real Marked/Entities AST + isolated cache/lifecycle/IPC. Never writes the system clipboard.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule, createContext } from 'node:vm';
import { webcrypto } from 'node:crypto';
import * as marked from 'marked';
import * as entities from 'entities';
const source = async file => stripTypeScriptTypes(await readFile(new URL(file, import.meta.url), 'utf8'), { mode: 'transform' });
const synthetic = (values, context) => new SyntheticModule(Object.keys(values), function () { for (const [key, value] of Object.entries(values)) this.setExport(key, value); }, context ? { context } : undefined);
const preview = new SourceTextModule(await source('table-preview.ts'));
await preview.link(name => synthetic(name === 'marked' ? marked : entities)); await preview.evaluate();
const { previewMessageTables, tablePlainText } = preview.namespace;
const resource = new SourceTextModule(await source('table-resource.ts')); await resource.link(() => { throw new Error('unexpected import'); }); await resource.evaluate();
const { TableMessageCache, TableRequestGate, tableTextHash } = resource.namespace;
const plain = value => JSON.parse(JSON.stringify(value));
const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const checks = [];

const markdown = '正文\n\n| 名称 | 空 | 注释 |\n| :--- | ---: | :---: |\n| [Mewu](https://example.test) | | `a\\|b` |\n| **中文** &amp; | 00123 | A<br>B |\n\n结束';
const document = previewMessageTables(markdown, 'hash');
assert.equal(document.tables.length, 1); assert.deepEqual(plain(document.tables[0]), { index: 0, header: ['名称', '空', '注释'], rows: [['Mewu', '', 'a|b'], ['中文 &', '00123', 'A\nB']], align: ['left', 'right', 'center'] });
assert.deepEqual(document.blocks.map(block => block.kind), ['markdown', 'table', 'markdown']);
assert.equal(previewMessageTables('```md\n| A |\n|---|\n|代码不是表格|\n```', 'hash').tables.length, 0);
checks.push('Actual Marked AST preserves empty cells, Unicode, escaped pipes, code/link labels, entities, alignment and br; fenced examples are not tables');

const nested = previewMessageTables('> | A | B |\n> |---|---|\n> |甲|乙|\n\n| C |\n|---|\n|尾表|', 'hash');
assert.equal(nested.tables.length, 2); assert.deepEqual(nested.tables.map(table => table.index), [0, 1]);
const literal = previewMessageTables('| 字段 |\n|---|\n| <b>可见</b><img src="https://example.test/x"> |\n| ![图像说明](https://example.test/a.png) |', 'hash');
assert.equal(literal.tables[0].rows[0][0], '可见'); assert.equal(literal.tables[0].rows[1][0], '图像说明');
assert.equal(previewMessageTables('普通正文 & <script>不会执行</script>', 'hash').blocks[0].text, '普通正文 & <script>不会执行</script>');
checks.push('Nested tables keep traversal indices; raw HTML never becomes cell markup and Markdown image alt stays text');

assert.throws(() => previewMessageTables(Array.from({ length: 13 }, () => '|A|\n|---|\n|B|').join('\n\n'), 'hash'), /超出/);
assert.throws(() => previewMessageTables('|A|\n|---|\n' + '|B|\n'.repeat(200), 'hash'), /超出/);
assert.throws(() => previewMessageTables('|' + 'A|'.repeat(33) + '\n|' + '---|'.repeat(33), 'hash'), /超出/);
assert.throws(() => previewMessageTables('|A|\n|---|\n|' + '文'.repeat(90_000) + '|', 'hash'), /超出/);
assert.equal(previewMessageTables('|A|\n|---|\n' + '|B|\n'.repeat(199), 'hash').tables[0].rows.length, 199);
checks.push('12-table, 200-total-row, 32-column and 256KiB-cell limits fail as a whole instead of truncating a purportedly complete table');

const format = { index: 0, header: ['甲', '乙'], rows: [['00123', 'a,"b"'], ['中文\n换行', '制\t表']], align: [null, 'right'] };
assert.equal(tablePlainText(format, 'csv'), '"甲","乙"\r\n"00123","a,""b"""\r\n"中文\n换行","制\t表"');
assert.equal(tablePlainText(format, 'tsv'), '"甲"\t"乙"\r\n"00123"\t"a,""b"""\r\n"中文\n换行"\t"制\t表"');
assert.ok(tablePlainText(document.tables[0], 'markdown').includes('a&#124;b'));
assert.equal(tablePlainText({ index: 0, header: ['原文'], rows: [['-12.50'], ['=SUM(A1:A2)'], ['+文本'], ['@原样']], align: [null] }, 'csv'), '"原文"\r\n"-12.50"\r\n"=SUM(A1:A2)"\r\n"+文本"\r\n"@原样"');
const punctuation = { index: 0, header: ['头部'], rows: [[' [字面](https://x) ~~保留~~ &amp; `代码` \\| \t\r\n行 ']], align: [null] };
assert.deepEqual(plain(previewMessageTables(tablePlainText(punctuation, 'markdown'), 'hash').tables[0]), punctuation);
checks.push('Text formats retain leading zeros, quote embedded delimiters/quotes/newlines and serialize Markdown escaped pipes');

const value = { sourceHash: 'h', blocks: [{ kind: 'markdown', text: 'text' }], tables: [] };
const cache = new TableMessageCache(), waits = [deferred(), deferred(), deferred()], started = [];
const pending = waits.map((wait, index) => cache.get(String(index), () => { started.push(index); return wait.promise; }));
assert.equal(cache.get('0', () => { throw new Error('duplicate'); }), pending[0]);
await tick(); assert.deepEqual(started, [0, 1]); waits[0].resolve(value); await tick(); assert.deepEqual(started, [0, 1, 2]);
waits[1].resolve(value); waits[2].resolve(value); await Promise.all(pending);
let attempts = 0; await assert.rejects(cache.get('fail', async () => { attempts++; throw new Error('host error'); }));
await cache.get('fail', async () => { attempts++; return value; }); assert.equal(attempts, 2);
const bytes = JSON.stringify(value).length * 2, small = new TableMessageCache(2, bytes + 1); let loaded = 0;
const get = key => small.get(key, async () => { loaded++; return value; });
await get('a'); await get('b'); await get('b'); assert.equal(loaded, 2); await get('a'); assert.equal(loaded, 3);
const saturated = new TableMessageCache(1), stuck = deferred(), first = saturated.get('first', () => stuck.promise);
await assert.rejects(saturated.get('second', async () => value), /繁忙/); stuck.resolve(value); await first;
checks.push('Cache deduplicates in-flight work, limits concurrency to two, bounds LRU by UTF-16 bytes/items and permits explicit retry after failure');

for (const action of ['cancel', 'dispose', 'replace']) {
  const gate = new TableRequestGate(), old = deferred(), accepted = [], errors = [];
  const pending = gate.load(() => old.promise, value => accepted.push(value.sourceHash), value => errors.push(value));
  if (action === 'replace') await gate.load(async () => ({ ...value, sourceHash: 'new' }), value => accepted.push(value.sourceHash), value => errors.push(value)); else gate[action]();
  old.resolve(value); await pending; assert.deepEqual(accepted, action === 'replace' ? ['new'] : []); assert.deepEqual(errors, []);
}
const rejected = new TableRequestGate(), failed = deferred(), errors = [];
const late = rejected.load(() => failed.promise, () => assert.fail('late'), value => errors.push(value)); rejected.dispose(); failed.reject(new Error('late')); await late; assert.deepEqual(errors, []);
assert.equal(await tableTextHash('中文'), await tableTextHash('中文')); assert.notEqual(await tableTextHash('中文'), await tableTextHash('中文 '));
checks.push('Scene/message replacement, unmount and late errors are ignored; cache text identity is a stable SHA-256 of the exact Unicode text');

async function bridgeFixture(native) {
  const calls = [], writes = [], context = createContext({ crypto: webcrypto, TextEncoder, Uint8Array, Map, Set, JSON, Promise, Error, navigator: { clipboard: { writeText: async value => { writes.push(value); } } } });
  const core = synthetic({ invoke: async (command, args) => { calls.push({ command, args }); return command === 'get_message_tables' ? { ...value, sourceHash: await tableTextHash(markdown) } : undefined; } }, context);
  const resource = new SourceTextModule(await source('table-resource.ts'), { context }); await resource.link(() => { throw new Error('unexpected resource import'); });
  const parse = synthetic({ previewMessageTables, tablePlainText }, context), mode = synthetic({ native }, context);
  const bridge = new SourceTextModule(await source('table-bridge.ts'), { context });
  await bridge.link(name => name.endsWith('/core') ? core : name === './bridge' ? mode : name === './table-resource' ? resource : parse); await bridge.evaluate();
  return { api: bridge.namespace, calls, writes };
}
const host = await bridgeFixture(true);
await Promise.all([host.api.getMessageTables('s', 'm', markdown), host.api.getMessageTables('s', 'm', markdown)]);
await host.api.exportMessageTable({ sceneId: 's', messageId: 'm', tableIndex: 2 }, 'table', 'not sent to native');
assert.deepEqual(plain(host.calls), [{ command: 'get_message_tables', args: { sceneId: 's', messageId: 'm' } }, { command: 'export_message_table', args: { sceneId: 's', messageId: 'm', tableIndex: 2, format: 'table' } }]);
await assert.rejects(host.api.getMessageTables('s', 'changed', 'changed source text'), /已更新/);
const browser = await bridgeFixture(false);
await assert.rejects(browser.api.exportMessageTable({ sceneId: 's', messageId: 'm', tableIndex: 0 }, 'table', markdown), /桌面版/);
await assert.rejects(browser.api.exportMessageTable({ sceneId: 's', messageId: 'm', tableIndex: 0 }, 'png', markdown), /桌面版/);
await browser.api.exportMessageTable({ sceneId: 's', messageId: 'm', tableIndex: 0 }, 'csv', markdown);
assert.equal(browser.calls.length, 0); assert.equal(browser.writes.length, 1); assert.ok(browser.writes[0].includes('00123'));
checks.push('Native bridge sends only persistent message identity/index/format, shares reads; preview rejects Windows/PNG claims while text writes use actual parsed cells');
console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
