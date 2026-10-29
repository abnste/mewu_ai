// SPDX-License-Identifier: MPL-2.0
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { lstat, realpath } from 'node:fs/promises';
import { readBounded } from './files.mjs';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
export const CONTRACT_PATHS = [
  'crates/mewu-extension/src/contract.rs',
  'crates/mewu-extension/src/package.rs',
  'crates/mewu-extension/src/sdk.js',
  'crates/mewu-extension/src/runtime.rs',
  'crates/mewu-extension/src/appearance.rs',
  'crates/mewu-extension/src/appearance-contract.json',
  'apps/desktop/src-tauri/src/extension_market.rs',
];

export async function extensionFingerprint(root = ROOT) {
  const hash = createHash('sha256');
  for (let index = 0; index < CONTRACT_PATHS.length; index++) {
    if (index) hash.update(Buffer.from([0]));
    hash.update(await readBounded(path.join(root, CONTRACT_PATHS[index]), 2 * 1024 * 1024));
  }
  return hash.digest('hex');
}
export async function projectFingerprint(root = ROOT) {
  return createHash('sha256').update(await readBounded(path.join(root, 'apps/desktop/src-tauri/src/extension_project.rs'), 2 * 1024 * 1024)).digest('hex');
}

export function invokeExtensionValidator(executable, mode, bytes, expectedFingerprint, timeoutMs = 15_000, project = false) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, [mode], { shell: false, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    const output = []; let size = 0, errors = 0, failure;
    const fail = message => { failure ??= new Error(message); child.kill('SIGKILL'); };
    const timer = setTimeout(() => fail('原生扩展校验器超时'), timeoutMs);
    child.stdout.on('data', chunk => { size += chunk.length; if (size > 4 * 1024 * 1024) fail('原生扩展校验器输出过多'); else output.push(chunk); });
    child.stderr.on('data', chunk => { errors += chunk.length; if (errors > 64 * 1024) fail('原生扩展校验器诊断过多'); });
    child.stdin.on('error', error => { if (error.code !== 'EPIPE') fail('无法向原生扩展校验器写入'); });
    child.on('error', () => { failure ??= new Error('无法启动原生扩展校验器'); });
    child.on('close', code => {
      clearTimeout(timer);
      if (failure) return reject(failure);
      let result;
      try { result = JSON.parse(Buffer.concat(output).toString('utf8')); }
      catch { return reject(new Error('原生扩展校验器没有返回有效 JSON')); }
      if (result?.protocolVersion !== 1 || (project
          ? result.projectContractVersion !== 1 || result.projectContractSha256 !== expectedFingerprint
          : result.extensionContractVersion !== 2 || result.extensionContractSha256 !== expectedFingerprint) || typeof result.ok !== 'boolean') {
        return reject(new Error('原生 API 契约与可信源码不一致，请重新构建校验器'));
      }
      if (code === 0 && result.ok && Object.hasOwn(result, 'value')) return resolve(result.value);
      if (code === 1 && !result.ok && typeof result.error === 'string') return reject(new Error(result.error));
      reject(new Error('原生扩展校验器异常退出'));
    });
    child.stdin.end(bytes);
  });
}

export async function createValidator(explicit) {
  const name = process.platform === 'win32' ? 'mewu-plugin-validator.exe' : 'mewu-plugin-validator';
  const filename = path.resolve(explicit ?? process.env.MEWU_PLUGIN_VALIDATOR ?? path.join(process.env.CARGO_TARGET_DIR ?? path.join(ROOT, 'target'), 'debug', name));
  if (path.basename(filename).toLowerCase() !== name.toLowerCase()) throw new Error(`仅接受独立 ${name} 校验器`);
  const info = await lstat(filename);
  if (!info.isFile() || info.isSymbolicLink()) throw new Error('校验器必须是普通文件');
  const executable = await realpath(filename), fingerprint = await extensionFingerprint();
  return async (mode, bytes) => {
    const project = ['extension-project', 'extension-projects', 'extension-project-image'].includes(mode);
    if (!project && !['extension-manifest', 'extension-entry', 'extension-catalog','extension-package'].includes(mode)) throw new Error('原生扩展校验模式无效');
    const expected = project ? await projectFingerprint() : fingerprint;
    const value = await invokeExtensionValidator(executable, mode, bytes, expected, 15_000, project);
    if ((project ? await projectFingerprint() : await extensionFingerprint()) !== expected) throw new Error('校验期间原生契约发生变化，请重试');
    return value;
  };
}
