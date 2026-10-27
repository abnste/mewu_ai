// SPDX-License-Identifier: MPL-2.0
// Opt-in installed-app evaluation. Uses the public CLI, never native private IPC.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { annotationEvidence, assertNativeRun } from './agent-eval-results.mjs';

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
const cliFailures = [];
function call(method, params = {}, wait = false) {
  return new Promise((resolve, reject) => {
    const child = spawn(cli, ['call', method, '--stdin', '--output', 'json', ...(wait ? ['--wait'] : [])], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    let stdout = '', stderr = '';
    const timeout = setTimeout(() => {
      cliFailures.push({ method, observationTimedOut: true, stdout, stderr });
      save('cli-failures.json', cliFailures);
      child.kill();
      reject(Object.assign(Error(`CLI observation timed out: ${method}; operation was NOT resent`), { executionUnknown: true }));
    }, 120_000);
    child.stdout.on('data', chunk => { stdout += chunk.toString(); });
    child.stderr.on('data', chunk => { stderr += chunk.toString(); });
    child.on('error', error => { clearTimeout(timeout); reject(error); });
    child.on('close', code => {
      clearTimeout(timeout);
      try {
        if (code !== 0) {
          cliFailures.push({ method, code, stdout, stderr });
          save('cli-failures.json', cliFailures);
          throw Object.assign(Error(`CLI ${method}: ${stderr || stdout}`), { cliExitCode: code });
        }
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
  if (args.includes('--open-space')) await call('capture.open', {}, true);
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
    // Activate only our new scene. --open-space separately opens the capture
    // surface; no captured desktop references are sent in these fixture tasks.
    await call('scene.activate', { sceneId: record.sceneId }, true);
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
    try {
      await call('chat.send', { context, text: test.prompt, refs: imported.map(item => ({ kind: 'item', id: item.itemId })) }, true);
    } catch (error) {
      if (error.cliExitCode !== 6 && !error.executionUnknown) throw error;
      // Unknown execution is not a retry signal (e.g. OS keyring confirmation).
      // Observe this newly-created scene only; preserve the original timeout.
      record.sendObservation = String(error);
      save('report.json', report);
    }
    let observed;
    for (let attempt = 0; attempt < 180; attempt++) {
      observed = await call('run.get', { sceneId: record.sceneId, limit: 12 });
      save(`case-${index}-run.json`, observed);
      if (observed.run?.externalRuntime) {
        // Cancel this exact run; never change the user's engine binding silently.
        if (observed.run.status === 'running') await call('chat.cancel', { context, expectedRunId: observed.run.id }, true);
      }
      assertNativeRun(observed.run, connectionId);
      if (observed.run && !['running', 'queued'].includes(observed.run.status)) break;
      await delay(3000);
    }
    record.runId = observed?.run?.id;
    record.status = observed?.run?.status;
    if (record.status !== 'completed') throw Error(`Case ${test.name}: run did not complete`);
    if (!observed.messages.some(message => message.role === 'user' && message.runId === record.runId && message.text === test.prompt)) throw Error('Observed run is not this evaluation prompt');
    const state = await call('scene.get', { sceneId: record.sceneId, limit: 12 });
    save(`case-${index}-scene.json`, state);
    Object.assign(record, annotationEvidence(state));
    record.steps = observed.run.steps;
    if (test.expectNoObjects && record.objectCount) throw Error(`${test.name}: text-only task mutated media`);
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
