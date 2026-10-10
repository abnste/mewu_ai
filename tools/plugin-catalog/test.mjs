// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import { invokeValidator, readBounded, run, writeAtomic } from './cli.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const validator = process.env.MEWU_PLUGIN_VALIDATOR;
const manifest = JSON.parse(await readFile(path.join(ROOT, 'apps/desktop/plugins/examples/selection-workflow/mewu-plugin.json'), 'utf8'));
const source = { type: 'github', repository: 'example/plugin', commit: 'a'.repeat(40), path: 'mewu-plugin.json' };

async function temporary(t) {
  const base = process.env.MEWU_PLUGIN_TEST_TMP ?? os.tmpdir();
  const directory = await mkdtemp(path.join(base, 'mewu-catalog-test-'));
  t.after(async () => {
    // Only the precise directory just created above is ever recursively removed.
    assert.equal(path.dirname(directory), path.resolve(base));
    await rm(directory, { recursive: true, force: true });
  });
  return directory;
}
async function invoke(args) {
  let stdout = '', stderr = '';
  const code = await run(args, { stdout: { write(s) { stdout += s; } }, stderr: { write(s) { stderr += s; } } });
  return { code, stdout, stderr };
}
const nativeArgs = () => ['--validator', validator, '--json'];

test('bounded reading rejects excess, directories and links', async t => {
  const directory = await temporary(t);
  const file = path.join(directory, 'input.json');
  await writeFile(file, '12345');
  assert.equal((await readBounded(file, 5)).toString(), '12345');
  await assert.rejects(readBounded(file, 4), /超过/);
  await assert.rejects(readBounded(directory, 10), /普通文件/);
  try { await symlink(file, path.join(directory, 'link.json')); }
  catch (error) { if (['EPERM', 'EACCES'].includes(error.code)) return; throw error; }
  await assert.rejects(readBounded(path.join(directory, 'link.json'), 10), /链接/);
});

test('atomic output replaces a regular file and refuses directory targets', async t => {
  const directory = await temporary(t);
  const file = path.join(directory, 'catalog.json');
  await writeFile(file, 'old');
  await writeAtomic(file, Buffer.from('new'));
  assert.equal(await readFile(file, 'utf8'), 'new');
  await mkdir(path.join(directory, 'blocked'));
  await assert.rejects(writeAtomic(path.join(directory, 'blocked'), Buffer.from('x')), /普通文件/);
  assert.deepEqual((await readdir(directory)).sort(), ['blocked', 'catalog.json']);
});

test('validator failure, stale source, timeout and non-JSON output cannot pass', async t => {
  const directory = await temporary(t);
  const script = path.join(directory, 'fixture.mjs');
  const response = { protocolVersion: 1, contractSha256: 'expected', ok: true, value: {} };
  await writeFile(script, `process.stdin.resume(); process.stdin.on('end',()=>console.log(${JSON.stringify(JSON.stringify(response))}));`);
  assert.deepEqual(await invokeValidator(process.execPath, script, Buffer.from('{}'), 'expected'), {});
  await assert.rejects(invokeValidator(process.execPath, script, Buffer.from('{}'), 'different'), /契约不一致/);
  await writeFile(script, 'process.stdin.resume();process.stdin.on("end",()=>console.log("not-json"));');
  await assert.rejects(invokeValidator(process.execPath, script, Buffer.from('{}'), 'expected'), /有效 JSON/);
  await writeFile(script, 'process.stdin.resume();setInterval(()=>{},1000);');
  await assert.rejects(invokeValidator(process.execPath, script, Buffer.from('{}'), 'expected', 100), /超时/);
});

test('model connection contract rejects secret injection, unknown protocols and unsafe endpoints through actual parser', { skip: !validator }, async t => {
  const directory = await temporary(t), file = path.join(directory, 'model.json');
  const value = JSON.parse(await readFile(path.join(ROOT,'apps/desktop/plugins/examples/model-connection/mewu-plugin.json'),'utf8'));
  const check = async candidate => { await writeFile(file,JSON.stringify(candidate)); return invoke(['manifest',file,...nativeArgs()]); };
  assert.equal((await check(value)).code,0);
  for (const field of ['script','command','apiKey','credentialId','headers','autoRun']) {
    const candidate = structuredClone(value); candidate.contributions[0].template[field] = 'injected';
    assert.notEqual((await check(candidate)).code,0,field);
  }
  for (const baseUrl of ['http://192.0.2.1/v1','https://user:secret@example.com/v1','https://example.com/v1?api-key=secret']) {
    const candidate = structuredClone(value); candidate.contributions[0].template.baseUrl = baseUrl;
    assert.notEqual((await check(candidate)).code,0,baseUrl);
  }
  for (const [protocol,requestParameters] of [['arbitrary',{}],['chat_completions',{tools:[]}],['chat_completions',{temperature:'1'}],['anthropic_messages',{temperature:1.1}],['anthropic_messages',{service_tier:'priority'}]]) {
    const candidate = structuredClone(value); Object.assign(candidate.contributions[0].template.advanced,{protocol,requestParameters});
    assert.notEqual((await check(candidate)).code,0,JSON.stringify({protocol,requestParameters}));
  }
});

test('actual host parser accepts supported kinds and denies executable or unknown capabilities', { skip: !validator }, async t => {
  const directory = await temporary(t);
  const file = path.join(directory, 'plugin.json');
  const accepted = [manifest,
    { ...manifest, contributions: [{ kind: 'model.connection', id: 'model', title: '本机模型', template: { providerId:'local', group:'custom', baseUrl:'http://127.0.0.1:1234/v1', model:'', advanced:{protocol:'openai_responses',authMode:'none',requestParameters:{temperature:0.5}} } }] },
    { ...manifest, contributions: [{ kind: 'agent.visual-annotations', id: 'annotate', title: '原位作答', engine: 'host.vector-v1' }] },
    { ...manifest, contributions: [{ kind: 'selection.recording', id: 'record', title: '屏幕录制', engine: 'windows.wgc-mf' }] },
    { ...manifest, contributions: [{ kind: 'artifact.video-gif', id: 'gif', title: 'GIF 导出', engine: 'windows.media-editing-gif' }] },
    { ...manifest, contributions: [{ kind: 'recording.audio', id: 'audio', title: '录屏声音', engine: 'windows.wasapi' }] },
    { ...manifest, contributions: [{ kind: 'artifact.video-trim', id: 'trim', title: '视频裁剪', engine: 'windows.media-editing' }] },
    { ...manifest, contributions: [{ kind: 'selection.codes', id: 'recognize', title: '二维码与条码', engine: 'rxing' }] },
    { ...manifest, contributions: [{ kind: 'selection.drawing-tools', id: 'draw', title: '绘制', tools: ['pen', 'arrow'] }] },
    { ...manifest, contributions: [{ kind: 'selection.ocr', id: 'ocr', title: '识别', engine: 'windows' }] },
    { ...manifest, contributions: [{ kind: 'selection.scroll', id: 'scroll', title: '长截图' }] },
    { ...manifest, contributions: [{ kind: 'selection.translation', id: 'translate', title: '原位翻译' }] },
    { ...manifest, contributions: [{ kind: 'selection.pin', id: 'pin', title: '贴图' }] },
    { ...manifest, contributions: [{ kind: 'memory.provider', id: 'memory', title: '记忆', adapter: 'hindsight' }] },
    { ...manifest, contributions: [{ kind: 'input.speech-to-text', id: 'dictation', title: '语音输入', engine: 'windows.sapi' }] }];
  for (const value of accepted) {
    await writeFile(file, JSON.stringify(value));
    const result = await invoke(['manifest', file, ...nativeArgs()]);
    assert.equal(result.code, 0, result.stdout);
  }
  const invalid = [
    ...['svg', 'html', 'host.vector-v2', { 'host.vector-v1': null }, ['host.vector-v1'], null, true].map(engine => ({
      ...manifest, contributions: [{ kind: 'agent.visual-annotations', id: 'annotate', title: '原位作答', engine }],
    })),
    ...['script', 'command', 'prompt', 'endpoint', 'tools', 'autoRun', 'replace', 'permissions'].map(field => ({
      ...manifest, contributions: [{ kind: 'agent.visual-annotations', id: 'annotate', title: '原位作答', engine: 'host.vector-v1', [field]: true }],
    })),
    ...['windows', 'ffmpeg', { 'windows.wgc-mf': null }, ['windows.wgc-mf'], null, true].map(engine => ({
      ...manifest, contributions: [{ kind: 'selection.recording', id: 'record', title: '屏幕录制', engine }],
    })),
    ...['autoStart', 'microphone', 'script', 'deviceId', 'command', 'permissions'].map(field => ({
      ...manifest, contributions: [{ kind: 'selection.recording', id: 'record', title: '屏幕录制', engine: 'windows.wgc-mf', [field]: true }],
    })),
    ...['ffmpeg', 'windows.media-editing', { 'windows.media-editing-gif': null }, ['windows.media-editing-gif'], null, true].map(engine => ({
      ...manifest, contributions: [{ kind: 'artifact.video-gif', id: 'gif', title: 'GIF 导出', engine }],
    })),
    ...['script', 'command', 'path', 'endpoint', 'codec', 'fps', 'range', 'loop', 'autoExport', 'permissions'].map(field => ({
      ...manifest, contributions: [{ kind: 'artifact.video-gif', id: 'gif', title: 'GIF 导出', engine: 'windows.media-editing-gif', [field]: true }],
    })),
    ...['wasapi', 'ffmpeg', { 'windows.wasapi': null }, ['windows.wasapi'], null, true].map(engine => ({
      ...manifest, contributions: [{ kind: 'recording.audio', id: 'audio', title: '录屏声音', engine }],
    })),
    ...['deviceId', 'microphone', 'autoStart', 'permissions', 'endpoint', 'command'].map(field => ({
      ...manifest, contributions: [{ kind: 'recording.audio', id: 'audio', title: '录屏声音', engine: 'windows.wasapi', [field]: true }],
    })),
    ...['ffmpeg', 'WindowsMediaEditing', { 'windows.media-editing': null }, ['windows.media-editing'], null, true].map(engine => ({
      ...manifest, contributions: [{ kind: 'artifact.video-trim', id: 'trim', title: '视频裁剪', engine }],
    })),
    ...['script', 'command', 'path', 'endpoint', 'codec', 'range', 'autoExport', 'permissions'].map(field => ({
      ...manifest, contributions: [{ kind: 'artifact.video-trim', id: 'trim', title: '视频裁剪', engine: 'windows.media-editing', [field]: 'forbidden' }],
    })),
    ...['remote', 'Rxing', {rxing:null}, ['rxing'], null, true].map(engine => ({
      ...manifest, contributions: [{ kind: 'selection.codes', id: 'recognize', title: 'Codes', engine }],
    })),
    ...['url', 'script', 'command', 'prompt', 'formats', 'autoOpen', 'permissions'].map(field => ({
      ...manifest, contributions: [{ kind: 'selection.codes', id: 'recognize', title: 'Codes', engine: 'rxing', [field]: 'unauthorized' }],
    })),
    { ...manifest, script: 'run.js' },
    { ...manifest, version: '1.0.0+UPPER' },
    { ...manifest, id: 'con.plugin' },
    { ...manifest, id: 'mewu.impersonation' },
    { ...manifest, license: 'AGPL-3.0' },
    { ...manifest, description: '字'.repeat(2001) },
    { ...manifest, contributions: [...manifest.contributions, ...manifest.contributions] },
    { ...manifest, contributions: [{ kind: 'selection.ocr', id: 'ocr', title: '识别', engine: 'remote' }] },
    { ...manifest, contributions: [{ kind: 'selection.drawing-tools', id: 'draw', title: '绘制', tools: ['pen', 'pen'] }] },
    { ...manifest, contributions: [{ ...manifest.contributions[0], accepts: ['image', 'image'] }] },
    ...['windows', 'WindowsSapi', 'windows.winrt', 'azure', 'whisper'].map(engine => ({
      ...manifest, contributions: [{ kind: 'input.speech-to-text', id: 'dictation', title: '听写', engine }],
    })),
    ...['script', 'endpoint', 'deviceId', 'audioPath', 'autoStart', 'language', 'command', 'prompt', 'tools'].map(field => ({
      ...manifest, contributions: [{ kind: 'input.speech-to-text', id: 'dictation', title: '听写', engine: 'windows.sapi', [field]: 'unauthorized' }],
    })),
    ...['script', 'url', 'windowUrl', 'html', 'command', 'prompt', 'tools'].map(field => ({
      ...manifest, contributions: [{ kind: 'selection.pin', id: 'pin', title: '贴图', [field]: 'unauthorized' }],
    })),
  ];
  for (const value of invalid) {
    await writeFile(file, JSON.stringify(value));
    const result = await invoke(['manifest', file, ...nativeArgs()]);
    assert.equal(result.code, 1, `should reject ${JSON.stringify(value)}: ${result.stdout}`);
  }
  for (const name of ['official-ocr.json', 'official-pin.json', 'official-voice.json', 'official-codes.json', 'migrations/official-recording-1.0.0.json']) {
    const official = path.join(ROOT, 'apps/desktop/plugins', name);
    assert.equal((await invoke(['manifest', official, ...nativeArgs()])).code, 1);
    assert.equal((await invoke(['manifest', official, '--bundled', ...nativeArgs()])).code, 0);
  }
});

test('raw duplicate keys, malformed UTF-8, BOM and byte limit reach the strict parser unchanged', { skip: !validator }, async t => {
  const directory = await temporary(t);
  const file = path.join(directory, 'plugin.json');
  const raw = JSON.stringify(manifest);
  const cases = [
    Buffer.from(`{"id":"other.id",${raw.slice(1)}`),
    Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(raw)]),
    Buffer.from('{"id":"\xff"}', 'latin1'),
    Buffer.alloc(65537, 32),
  ];
  for (const bytes of cases) {
    await writeFile(file, bytes);
    assert.equal((await invoke(['manifest', file, ...nativeArgs()])).code, 1);
  }
});

test('typed capability declarations use the same strict parser inside community entries and catalogs', { skip: !validator }, async t => {
  const directory = await temporary(t);
  const file = path.join(directory, 'speech.json');
  const speech = { ...manifest, id: 'example.voice', contributions: [{ kind: 'input.speech-to-text', id: 'dictation', title: '语音输入', engine: 'windows.sapi' }] };
  const codes = { ...manifest, id: 'example.codes', contributions: [{ kind: 'selection.codes', id: 'recognize', title: '二维码与条码', engine: 'rxing' }] };
  const video = { ...manifest, id: 'example.video-trim', contributions: [{ kind: 'artifact.video-trim', id: 'trim', title: '视频裁剪', engine: 'windows.media-editing' }] };
  const audio = { ...manifest, id: 'example.recording-audio', contributions: [{ kind: 'recording.audio', id: 'audio', title: '录屏声音', engine: 'windows.wasapi' }] };
  const gif = { ...manifest, id: 'example.gif', contributions: [{ kind: 'artifact.video-gif', id: 'gif', title: 'GIF 导出', engine: 'windows.media-editing-gif' }] };
  const recording = { ...JSON.parse(await readFile(path.join(ROOT, 'apps/desktop/plugins/migrations/official-recording-1.0.0.json'), 'utf8')), id: 'example.recording' };
  const annotations = { ...JSON.parse(await readFile(path.join(ROOT, 'apps/desktop/plugins/migrations/official-annotations-1.0.0.json'), 'utf8')), id: 'example.annotations' };
  const cases = [
    [annotations, true],
    [{ ...annotations, id: 'mewu.annotations' }, false],
    ...[{ 'host.vector-v1': null }, ['host.vector-v1'], null, true, 'html'].map(engine => [
      { ...annotations, contributions: [{ ...annotations.contributions[0], engine }] }, false,
    ]),
    ...['autoRun', 'prompt', 'replace', 'endpoint', 'permissions'].map(field => [
      { ...annotations, contributions: [{ ...annotations.contributions[0], [field]: true }] }, false,
    ]),
    [recording, true],
    [{ ...recording, id: 'mewu.recording' }, false],
    [{ ...recording, contributions: [{ ...recording.contributions[0], autoStart: true }] }, false],
    [{ ...recording, contributions: [{ ...recording.contributions[0], engine: { 'windows.wgc-mf': null } }] }, false],
    [gif, true],
    [{ ...gif, id: 'mewu.gif' }, false],
    ...[{ 'windows.media-editing-gif': null }, ['windows.media-editing-gif'], null, true, 'ffmpeg'].map(engine => [
      { ...gif, contributions: [{ ...gif.contributions[0], engine }] }, false,
    ]),
    [{ ...gif, contributions: [{ ...gif.contributions[0], autoExport: true }] }, false],
    [audio, true],
    [{ ...audio, id: 'mewu.recording-audio' }, false],
    ...[{ 'windows.wasapi': null }, ['windows.wasapi'], null, true, 'ffmpeg'].map(engine => [
      { ...audio, contributions: [{ ...audio.contributions[0], engine }] }, false,
    ]),
    [{ ...audio, contributions: [{ ...audio.contributions[0], autoStart: true }] }, false],
    [video, true],
    [{ ...video, id: 'mewu.video-trim' }, false],
    ...[{ 'windows.media-editing': null }, ['windows.media-editing'], null, true, 'ffmpeg'].map(engine => [
      { ...video, contributions: [{ ...video.contributions[0], engine }] }, false,
    ]),
    [{ ...video, contributions: [{ ...video.contributions[0], command: 'ffmpeg' }] }, false],
    [codes, true],
    [{ ...codes, id: 'mewu.codes' }, false],
    ...[{ rxing: null }, ['rxing'], null, true, 'remote'].map(engine => [
      { ...codes, contributions: [{ ...codes.contributions[0], engine }] }, false,
    ]),
    [{ ...codes, contributions: [{ ...codes.contributions[0], autoOpen: true }] }, false],
    [speech, true],
    [{ ...speech, id: 'mewu.voice' }, false],
    ...[{ 'windows.sapi': null }, ['windows.sapi'], null, true].map(engine => [
      { ...speech, contributions: [{ ...speech.contributions[0], engine }] }, false,
    ]),
    [{ ...speech, contributions: [{ ...speech.contributions[0], autoStart: true }] }, false],
  ];
  for (const [candidate, accepted] of cases) {
    const entry = { manifest: candidate, source };
    await writeFile(file, JSON.stringify(entry));
    assert.equal((await invoke(['entry', file, ...nativeArgs()])).code, accepted ? 0 : 1);
    await writeFile(file, JSON.stringify({ schemaVersion: 1, plugins: [entry] }));
    assert.equal((await invoke(['catalog', file, ...nativeArgs()])).code, accepted ? 0 : 1);
  }
});

test('catalog generation is deterministic, rejects bad sources and preserves last good output on failure', { skip: !validator }, async t => {
  const directory = await temporary(t);
  const entries = path.join(directory, 'entries');
  const output = path.join(directory, 'mewu-catalog.json');
  await mkdir(entries);
  const first = { manifest: { ...manifest, id: 'z.chart' }, source };
  const second = { manifest: { ...manifest, id: 'a.chart' }, source: { ...source, repository: 'example/second' } };
  const file = path.join(entries, 'first.json');
  await writeFile(file, JSON.stringify(first));
  await writeFile(path.join(entries, 'second.json'), JSON.stringify(second));
  const args = ['build', entries, '--out', output, ...nativeArgs()];
  const built = await invoke(args);
  assert.equal(built.code, 0, built.stdout);
  const saved = await readFile(output);
  assert.deepEqual(JSON.parse(saved).plugins.map(entry => entry.manifest.id), ['a.chart', 'z.chart']);
  assert.equal((await invoke([...args, '--check'])).code, 0);
  assert.equal((await invoke(['catalog', output, ...nativeArgs()])).code, 0);
  for (const bad of [
    { ...first, source: { ...source, commit: 'main' } },
    { ...first, source: { type: 'official' } },
    { ...first, source: { ...source, path: 'file.json?credential=x' } },
    { ...first, source: { ...source, path: '../mewu-plugin.json' } },
    { ...first, manifest: second.manifest },
  ]) {
    await writeFile(file, JSON.stringify(bad));
    assert.equal((await invoke(args)).code, 1);
    assert.deepEqual(await readFile(output), saved);
  }
  await writeFile(file, JSON.stringify({ ...first, manifest: { ...first.manifest, name: '已变更' } }));
  assert.equal((await invoke([...args, '--check'])).code, 1);
  assert.deepEqual(await readFile(output), saved);
  await writeFile(path.join(entries, 'README.md'), 'misplaced');
  assert.match((await invoke(args)).stdout, /不会静默忽略/);
  assert.deepEqual(await readFile(output), saved);
  assert.equal((await readdir(directory)).some(name => name.startsWith('.mewu-catalog-')), false);
  assert.equal((await invoke(['build', entries, '--out', path.join(entries, 'output.json'), ...nativeArgs()])).code, 1);
});

test('usage mistakes never try to launch a desktop app', async () => {
  assert.equal((await invoke(['--help'])).code, 0);
  assert.equal((await invoke(['manifest'])).code, 2);
  assert.equal((await invoke(['build', '.', '--check'])).code, 2);
  assert.equal((await invoke(['catalog', 'missing', '--bundled'])).code, 2);
});
