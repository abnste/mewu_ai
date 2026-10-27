// SPDX-License-Identifier: MPL-2.0
// Opt-in installed-app evaluation. Uses the public CLI, never native private IPC.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

const args = process.argv.slice(2);
if (!args.includes('--allow-model-requests')) throw Error('This live evaluation sends model requests; pass --allow-model-requests explicitly.');
const option = name => { const i = args.indexOf(name); if (i < 0 || !args[i + 1]) throw Error(`Missing ${name}`); return args[i + 1]; };
const cli = path.resolve(option('--cli')), manifestPath = path.resolve(option('--cases'));
const output = path.resolve(option('--output')), agentId = option('--agent'), connectionId = option('--connection');
const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
if (!Array.isArray(manifest.cases) || !manifest.cases.length || manifest.cases.length > 20) throw Error('Expected 1–20 cases');
if (fs.existsSync(output)) throw Error('Use a fresh evidence directory; never overwrite a failed run');
fs.mkdirSync(output, { recursive: true });
const save = (name, value) => fs.writeFileSync(path.join(output, name), `${JSON.stringify(value, null, 2)}\n`);
function call(method, params = {}, wait = false) {
  return new Promise((resolve, reject) => {
    const child = spawn(cli, ['call', method, '--stdin', '--output', 'json', ...(wait ? ['--wait'] : [])], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    let stdout = '', stderr = '';
    const timeout = setTimeout(() => { child.kill(); reject(Error(`CLI observation timed out: ${method}; operation was NOT resent`)); }, 120_000);
    child.stdout.on('data', chunk => { stdout += chunk.toString(); });
    child.stderr.on('data', chunk => { stderr += chunk.toString(); });
    child.on('error', error => { clearTimeout(timeout); reject(error); });
    child.on('close', code => {
      clearTimeout(timeout);
      try {
        if (code !== 0) throw Error(`CLI ${method}: ${stderr || stdout}`);
        const response = JSON.parse(stdout);
        if (response.error) throw Error(JSON.stringify(response.error));
        const result = response.result;
        if (wait && (result.status !== 'completed' || result.error)) throw Error(JSON.stringify(result));
        resolve(wait ? result.result : result);
      } catch (error) { reject(error); }
    });
    child.stdin.end(JSON.stringify(params));
  });
}
const report = { startedAt: new Date().toISOString(), platform: process.platform, agentId, connectionId, guiVerified: false, cases: [], passed: false };
try {
  report.host = await call('app.status');
  save('host.json', report.host);
  // Caller records source/package identity separately; a version string alone
  // cannot certify which commit produced the installed binary.
  for (const [index, test] of manifest.cases.entries()) {
    const record = { name: test.name, passed: false };
    report.cases.push(record);
    if (typeof test.prompt !== 'string' || !test.prompt.trim()) throw Error('Missing case prompt');
    const scene = await call('scene.new', {}, true);
    record.sceneId = scene.sceneId;
    if (!record.sceneId) throw Error('New scene did not return its identity');
    const context = { sceneId: record.sceneId };
    await call('scene.command', { context, command: { type: 'set_agent', sceneId: record.sceneId, agentId } }, true);
    await call('scene.command', { context, command: { type: 'set_scene_connection', sceneId: record.sceneId, connectionId } }, true);
    let imported = [];
    if (test.files?.length) {
      const paths = test.files.map(file => {
        const resolved = path.resolve(path.dirname(manifestPath), file);
        if (!fs.statSync(resolved).isFile()) throw Error('Fixture is not a file');
        return resolved;
      });
      imported = (await call('asset.import', { context, paths }, true)).items;
    }
    await call('chat.send', { context, text: test.prompt, refs: imported.map(item => ({ kind: 'item', id: item.itemId })) }, true);
    let observed;
    for (let attempt = 0; attempt < 180; attempt++) {
      observed = await call('run.get', { sceneId: record.sceneId, limit: 12 });
      save(`case-${index}-run.json`, observed);
      if (observed.run && !['running', 'queued'].includes(observed.run.status)) break;
      await delay(3000);
    }
    record.runId = observed?.run?.id;
    record.status = observed?.run?.status;
    if (record.status !== 'completed') throw Error(`Case ${test.name}: run did not complete`);
    const state = await call('scene.get', { sceneId: record.sceneId, limit: 12 });
    save(`case-${index}-scene.json`, state);
    const objects = state.items.flatMap(item => [ ...(item.imageAnnotations?.drawings || []), ...(item.videoAnnotations?.objects || []) ]);
    record.objectCount = objects.length;
    record.kinds = objects.map(item => item.kind || item.primitive?.kind);
    record.steps = observed.run.steps;
    if (test.expectNoObjects && objects.length) throw Error(`${test.name}: text-only task mutated media`);
    for (const kind of test.expectKinds || []) if (!record.kinds.includes(kind)) throw Error(`${test.name}: missing actual ${kind} object`);
    for (const name of test.expectTools || []) if (!record.steps.some(step => step.name === name && step.status === 'completed')) throw Error(`${test.name}: missing successful ${name}`);
    if (test.expectText && !state.messages.some(message => message.role === 'assistant' && message.text.includes(test.expectText))) throw Error(`${test.name}: expected answer missing`);
    record.passed = true;
    save('report.json', report);
    console.log(JSON.stringify({ name: record.name, sceneId: record.sceneId, passed: true, objectCount: record.objectCount }));
  }
  report.passed = true;
} catch (error) {
  report.error = String(error);
  process.exitCode = 1;
} finally {
  report.finishedAt = new Date().toISOString();
  save('report.json', report);
  console.log(JSON.stringify({ passed: report.passed, cases: report.cases.length, error: report.error }));
}
