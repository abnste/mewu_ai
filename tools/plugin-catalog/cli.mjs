// SPDX-License-Identifier: MPL-2.0
import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { constants } from 'node:fs';
import { lstat, open, readdir, realpath, rename, unlink } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const CATALOG_LIMIT = 2 * 1024 * 1024;
const MANIFEST_LIMIT = 64 * 1024;
const OUTPUT_LIMIT = 4 * 1024 * 1024;
const HELP = `Mewu 插件与社区目录校验（Node.js 22+，完全离线）

node tools/plugin-catalog/cli.mjs manifest <plugin.json> [--bundled] [--json]
node tools/plugin-catalog/cli.mjs entry <entry.json> [--json]
node tools/plugin-catalog/cli.mjs catalog <mewu-catalog.json> [--json]
node tools/plugin-catalog/cli.mjs build <entries目录> --out <目录.json> [--check] [--json]

所有命令可加 --validator <mewu-plugin-validator可执行文件>。
也可设置 MEWU_PLUGIN_VALIDATOR；否则从 CARGO_TARGET_DIR/debug 或 target/debug 查找。
构建校验器：cargo build -p mewu-desktop --bin mewu-plugin-validator --locked
--bundled 只检查内置清单格式，不赋予任何文件官方来源权限。
--check 比较生成结果，不写文件。目录仅接收直属 JSON 条目，不静默略过其他文件。
`;

export class ToolError extends Error {
  constructor(message, exitCode = 1) { super(message); this.exitCode = exitCode; }
}

export async function readBounded(filename, limit) {
  const before = await lstat(filename);
  if (!before.isFile() || before.isSymbolicLink()) throw new ToolError(`必须是普通文件，不能是链接：${filename}`);
  if (before.size > limit) throw new ToolError(`文件超过 ${limit} 字节：${filename}`);
  const handle = await open(filename, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const current = await handle.stat();
    if (!current.isFile() || before.ino !== current.ino || before.dev !== current.dev) {
      throw new ToolError(`读取时文件已被替换：${filename}`);
    }
    const bytes = Buffer.alloc(limit + 1);
    let size = 0;
    while (size < bytes.length) {
      const { bytesRead } = await handle.read(bytes, size, bytes.length - size, null);
      if (bytesRead === 0) break;
      size += bytesRead;
    }
    if (size > limit) throw new ToolError(`文件超过 ${limit} 字节：${filename}`);
    return bytes.subarray(0, size);
  } finally { await handle.close(); }
}

function parseArguments(args) {
  if (args.length === 0 || (args.length === 1 && ['--help', '-h'].includes(args[0]))) return { help: true };
  const [kind, input, ...rest] = args;
  if (!['manifest', 'entry', 'catalog', 'build'].includes(kind) || !input || input.startsWith('--')) {
    throw new ToolError('请指定校验模式和输入路径；使用 --help 查看用法', 2);
  }
  const options = { kind, input: path.resolve(input), json: false, check: false, bundled: false };
  const used = new Set();
  for (let i = 0; i < rest.length; i++) {
    const flag = rest[i];
    if (used.has(flag)) throw new ToolError(`重复参数：${flag}`, 2);
    used.add(flag);
    if (['--json', '--check', '--bundled'].includes(flag)) options[flag.slice(2)] = true;
    else if (['--validator', '--out'].includes(flag) && rest[i + 1] && !rest[i + 1].startsWith('--')) {
      options[flag.slice(2)] = path.resolve(rest[++i]);
    } else throw new ToolError(`未知或缺值参数：${flag}`, 2);
  }
  if (options.bundled && kind !== 'manifest') throw new ToolError('--bundled 仅用于 manifest', 2);
  if (kind !== 'build' && (options.check || options.out)) throw new ToolError('--out/--check 仅用于 build', 2);
  if (kind === 'build' && !options.out) throw new ToolError('build 必须明确 --out 输出文件', 2);
  return options;
}

async function sourceFingerprint() {
  const hash = createHash('sha256');
  hash.update(await readBounded(path.join(ROOT, 'apps/desktop/src-tauri/src/plugins.rs'), 1024 * 1024));
  hash.update(Buffer.from([0]));
  hash.update(await readBounded(path.join(ROOT, 'apps/desktop/src-tauri/src/plugin_sources.rs'), 1024 * 1024));
  hash.update(Buffer.from([0]));
  hash.update(await readBounded(path.join(ROOT, 'crates/mewu-core/src/model.rs'), 1024 * 1024));
  hash.update(Buffer.from([0]));
  hash.update(await readBounded(path.join(ROOT, 'crates/mewu-core/src/connections.rs'), 1024 * 1024));
  return hash.digest('hex');
}

async function validatorPath(explicit) {
  const name = process.platform === 'win32' ? 'mewu-plugin-validator.exe' : 'mewu-plugin-validator';
  const file = explicit ?? process.env.MEWU_PLUGIN_VALIDATOR
    ?? path.join(process.env.CARGO_TARGET_DIR ?? path.join(ROOT, 'target'), 'debug', name);
  const resolved = path.resolve(file);
  if (path.basename(resolved).toLowerCase() !== name.toLowerCase()) {
    throw new ToolError(`必须使用名为 ${name} 的独立校验器；不要传入桌面应用或脚本`, 2);
  }
  try {
    const info = await lstat(resolved);
    if (!info.isFile() || info.isSymbolicLink()) throw new Error();
    return await realpath(resolved);
  } catch {
    throw new ToolError(`未找到独立校验器：${resolved}。请先构建 mewu-plugin-validator，不能使用 Mewu 桌面 EXE 代替。`, 2);
  }
}

export function invokeValidator(executable, kind, bytes, expectedFingerprint, timeoutMs = 15_000) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, [kind], { shell: false, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    let output = [], size = 0, errorSize = 0, failure;
    const fail = (message) => {
      failure ??= new ToolError(message, 2);
      child.kill('SIGKILL');
    };
    const timer = setTimeout(() => fail('校验器超时，未生成任何目录文件'), timeoutMs);
    child.stdout.on('data', chunk => {
      size += chunk.length;
      if (size > OUTPUT_LIMIT) fail('校验器输出超过限制');
      else output.push(chunk);
    });
    // Consume stderr to avoid pipe deadlock, without echoing arbitrary text.
    child.stderr.on('data', chunk => { errorSize += chunk.length; if (errorSize > 64 * 1024) fail('校验器诊断输出超过限制'); });
    child.stdin.on('error', error => { if (error.code !== 'EPIPE') fail('无法将输入传给校验器'); });
    child.on('error', () => { failure ??= new ToolError('无法启动独立校验器', 2); });
    child.on('close', code => {
      clearTimeout(timer);
      if (failure) return reject(failure);
      let result;
      try { result = JSON.parse(Buffer.concat(output).toString('utf8')); }
      catch { return reject(new ToolError('校验器未返回有效 JSON；请构建当前版本的独立校验器', 2)); }
      if (result?.protocolVersion !== 1 || typeof result.ok !== 'boolean'
          || result.contractSha256 !== expectedFingerprint) {
        return reject(new ToolError('校验器与当前工作区契约不一致，请重新构建 mewu-plugin-validator', 2));
      }
      if (code === 0 && result.ok && Object.hasOwn(result, 'value')) return resolve(result.value);
      if (code === 1 && !result.ok && typeof result.error === 'string') return reject(new ToolError(result.error));
      return reject(new ToolError('校验器异常退出，结果未被接受', 2));
    });
    child.stdin.end(bytes);
  });
}

function isWithin(directory, filename) {
  const relative = path.relative(directory, filename);
  return relative === '' || (!relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative));
}

export async function writeAtomic(filename, bytes) {
  const directory = await realpath(path.dirname(filename));
  const target = path.join(directory, path.basename(filename));
  try {
    const existing = await lstat(target);
    if (!existing.isFile() || existing.isSymbolicLink()) throw new ToolError('输出目标必须是普通文件，不能是链接或目录');
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
  const temporary = path.join(directory, `.mewu-catalog-${randomUUID()}.tmp`);
  let handle;
  try {
    handle = await open(temporary, 'wx', 0o600);
    await handle.writeFile(bytes);
    await handle.sync();
    await handle.close();
    handle = undefined;
    // Same-directory rename: never delete a previous catalog before replacing.
    // If Windows refuses a locked target, leave it intact and report failure.
    await rename(temporary, target);
  } finally {
    await handle?.close();
    await unlink(temporary).catch(error => { if (error.code !== 'ENOENT') throw error; });
  }
}

async function build(options, validate) {
  const inputInfo = await lstat(options.input);
  if (!inputInfo.isDirectory() || inputInfo.isSymbolicLink()) throw new ToolError('条目路径必须是普通目录');
  const directory = await realpath(options.input);
  const output = path.join(await realpath(path.dirname(options.out)), path.basename(options.out));
  if (isWithin(directory, output)) throw new ToolError('输出文件必须位于条目目录之外，避免把生成物再次当条目收录');
  const files = await readdir(directory, { withFileTypes: true });
  if (files.length > 256) throw new ToolError('目录最多包含 256 个条目');
  files.sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
  const entries = [];
  let total = 0;
  for (const file of files) {
    if (!file.isFile() || file.isSymbolicLink() || !file.name.endsWith('.json')) {
      throw new ToolError(`条目目录只能放直属 .json 普通文件，不会静默忽略：${file.name}`);
    }
    const bytes = await readBounded(path.join(directory, file.name), CATALOG_LIMIT);
    total += bytes.length;
    if (total > CATALOG_LIMIT) throw new ToolError('所有条目的原始输入总计超过 2 MiB');
    try { entries.push(await validate('entry', bytes)); }
    catch (error) { throw new ToolError(`${file.name}：${error.message}`, error.exitCode); }
  }
  entries.sort((a, b) => a.manifest.id < b.manifest.id ? -1 : a.manifest.id > b.manifest.id ? 1 : 0);
  const bytes = Buffer.from(`${JSON.stringify({ schemaVersion: 1, plugins: entries }, null, 2)}\n`, 'utf8');
  if (bytes.length > CATALOG_LIMIT) throw new ToolError('生成的目录超过 2 MiB');
  await validate('catalog', bytes); // Actual host parser checks cross-entry IDs.
  if (options.check) {
    let current;
    try { current = await readBounded(output, CATALOG_LIMIT); }
    catch (error) {
      if (error.code === 'ENOENT') throw new ToolError('生成目录不存在，请先执行不带 --check 的 build');
      throw error;
    }
    if (!current.equals(bytes)) throw new ToolError('目录与条目不一致，请重新执行 build');
  } else await writeAtomic(output, bytes);
  return { count: entries.length, output, bytes: bytes.length, checked: options.check };
}

export async function run(args, io = { stdout: process.stdout, stderr: process.stderr }) {
  const wantsJson = args.includes('--json');
  try {
    const options = parseArguments(args);
    if (options.help) { io.stdout.write(HELP); return 0; }
    const executable = await validatorPath(options.validator);
    const fingerprint = await sourceFingerprint();
    const validate = async (kind, bytes) => {
      const value = await invokeValidator(executable, kind, bytes, fingerprint);
      if (await sourceFingerprint() !== fingerprint) {
        throw new ToolError('校验期间宿主契约发生变化，请重新构建校验器后重试', 2);
      }
      return value;
    };
    let details;
    if (options.kind === 'build') details = await build(options, validate);
    else {
      const mode = options.bundled ? 'bundled-manifest' : options.kind;
      const bytes = await readBounded(options.input, options.kind === 'manifest' ? MANIFEST_LIMIT : CATALOG_LIMIT);
      const result = await validate(mode, bytes);
      details = options.kind === 'catalog' ? { count: result.plugins.length }
        : { id: (result.manifest ?? result).id, version: (result.manifest ?? result).version };
    }
    const result = { ok: true, kind: options.kind, contractSha256: fingerprint, ...details };
    io.stdout.write(options.json ? `${JSON.stringify(result)}\n` : `校验通过：${details.id ?? `${details.count} 个条目`}${options.check ? '（生成结果一致）' : ''}\n`);
    return 0;
  } catch (error) {
    const message = error instanceof ToolError ? error.message : `${error.code ?? '错误'}：${error.message}`;
    (wantsJson ? io.stdout : io.stderr).write(wantsJson ? `${JSON.stringify({ ok: false, error: message })}\n` : `校验失败：${message}\n`);
    return error.exitCode ?? 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  process.exitCode = await run(process.argv.slice(2));
}
