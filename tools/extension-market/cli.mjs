// SPDX-License-Identifier: MPL-2.0
import { spawn } from 'node:child_process';
import { lstat, readdir, realpath } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { readBounded, writeAtomic } from '../plugin-catalog/cli.mjs';
import { createValidator } from './validator.mjs';
import { createZip, MAX_EXPANDED_BYTES, MAX_PACKAGE_BYTES, packagePath, readZip } from './zip.mjs';
import { bindEntryManifest, downloadReleaseAsset, getRelease, sha256, verifyEntry } from './github.mjs';
import { buildProjects, bindProjectEntry, verifyProject } from './projects.mjs';

const MAX_CATALOG = 2 * 1024 * 1024;
const HELP = `Mewu API 2 JavaScript 插件与 GitHub 社区市场（Node.js 22+）

node tools/extension-market/cli.mjs manifest <mewu-plugin.json>
node tools/extension-market/cli.mjs entry <entry.json>
node tools/extension-market/cli.mjs catalog <mewu-catalog.json>
node tools/extension-market/cli.mjs verify <entry.json>
node tools/extension-market/cli.mjs project <project.json>
node tools/extension-market/cli.mjs projects <mewu-projects.json>
node tools/extension-market/cli.mjs verify-project <project.json> --catalog <mewu-catalog.json>
node tools/extension-market/cli.mjs build-projects <projects> --catalog <mewu-catalog.json> --out <mewu-projects.json> [--previous <mewu-projects.json>] [--check] [--offline]
node tools/extension-market/cli.mjs build <entries> --out <catalog.json> [--previous <catalog.json>] [--check] [--offline]
node tools/extension-market/cli.mjs pack <plugin-directory> --out <package.zip> [--file <relative.js> ...]
node tools/extension-market/cli.mjs publish <package.zip> --repository <owner/repo> --release-tag <tag> --entry-out <entry.json> [--platform <platform> ...]

所有命令支持 --validator <独立校验器>、--json。JSON 原始字节交给宿主的同一严格 Rust parser。
build 默认实际核验 GitHub 发行 ZIP，全部通过后原子替换目录；--offline 仅用于开发。
pack 默认仅包含根 mewu-plugin.json 和清单 entry，其他 JS 模块须逐个 --file 明确加入。
publish 明确上传到现有已发布 GitHub release，不创建仓库、tag 或 release，不覆盖同名资产。
`;

function within(directory, filename) {
  const relative = path.relative(directory, filename);
  return relative === '' || (!relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative));
}

function optionsFor(args) {
  if (args.length === 0 || (args.length === 1 && ['--help', '-h'].includes(args[0]))) return { help: true };
  const [kind, input, ...flags] = args;
  if (!['manifest', 'entry', 'catalog', 'verify', 'build', 'pack', 'publish', 'project', 'projects', 'verify-project', 'build-projects'].includes(kind) || !input || input.startsWith('--')) throw new Error('指定命令和输入路径；使用 --help 查看用法');
  const result = { kind, input: path.resolve(input), files: [], platforms: [], json: false, offline: false, check: false }, seen = new Set();
  const allowed = new Set(['--validator', '--json', ...({ build: ['--out', '--previous', '--check', '--offline'], 'build-projects': ['--catalog', '--out', '--previous', '--check', '--offline'], 'verify-project': ['--catalog'], pack: ['--out', '--file'], publish: ['--repository', '--release-tag', '--entry-out', '--platform'] }[kind] ?? [])]);
  for (let at = 0; at < flags.length; at++) {
    const flag = flags[at];
    if (!allowed.has(flag) || (seen.has(flag) && !['--file', '--platform'].includes(flag))) throw new Error(`无效或重复参数：${flag}`);
    seen.add(flag);
    if (['--json', '--offline', '--check'].includes(flag)) result[flag.slice(2)] = true;
    else {
      const value = flags[++at];
      if (!value || value.startsWith('--')) throw new Error(`参数缺值：${flag}`);
      if (flag === '--file') result.files.push(value);
      else if (flag === '--platform') result.platforms.push(value);
      else result[flag.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = value;
    }
  }
  if (['pack', 'build', 'build-projects'].includes(kind) && !result.out) throw new Error(`${kind} 必须指定 --out`);
  if (['build-projects', 'verify-project'].includes(kind) && !result.catalog) throw new Error(`${kind} 必须指定 --catalog`);
  if (kind === 'publish' && (!result.repository || !result.releaseTag || !result.entryOut)) throw new Error('publish 必须指定 --repository、--release-tag、--entry-out');
  for (const key of ['out', 'previous', 'entryOut', 'validator', 'catalog']) if (result[key]) result[key] = path.resolve(result[key]);
  return result;
}

export async function buildCatalog(options, validate, verify = verifyEntry) {
  const info = await lstat(options.input);
  if (!info.isDirectory() || info.isSymbolicLink()) throw new Error('条目路径必须是普通目录');
  const directory = await realpath(options.input), output = path.join(await realpath(path.dirname(options.out)), path.basename(options.out));
  if (within(directory, output)) throw new Error('输出不能放在 entries 条目目录内');
  const files = await readdir(directory, { withFileTypes: true });
  if (files.length > 256) throw new Error('社区目录最多 256 个插件');
  const entries = []; let total = 0;
  for (const file of files.sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
    if (!file.isFile() || file.isSymbolicLink() || !file.name.endsWith('.json')) throw new Error('entries 只能包含直属 .json 普通文件');
    const raw = await readBounded(path.join(directory, file.name), MAX_CATALOG);
    total += raw.length; if (total > MAX_CATALOG) throw new Error('条目总计超过 2 MiB');
    const entry = await validate('extension-entry', raw);
    if (file.name !== `${entry.id}.json`) throw new Error('条目文件名必须等于插件 id.json');
    entries.push(entry);
  }
  entries.sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  const bytes = Buffer.from(`${JSON.stringify({ schemaVersion: 2, plugins: entries }, null, 2)}\n`, 'utf8');
  if (bytes.length > MAX_CATALOG) throw new Error('生成目录超过 2 MiB');
  const catalog = await validate('extension-catalog', bytes);
  let previous;
  try { previous = await validate('extension-catalog', await readBounded(options.previous ?? output, MAX_CATALOG)); }
  catch (error) { if (error.code !== 'ENOENT') throw error; if (options.previous) throw new Error('指定的可信旧目录不存在'); }
  if (previous) {
    const prior = new Map(previous.plugins.map(entry => [entry.id, entry]));
    for (const entry of catalog.plugins) {
      const old = prior.get(entry.id);
      if (old && old.repository.toLowerCase() !== entry.repository.toLowerCase()) throw new Error(`禁止把已有插件 ${entry.id} 改到其他 GitHub 来源`);
      if (old && old.version === entry.version && (old.sha256 !== entry.sha256 || old.size !== entry.size)) throw new Error(`同一版本的插件 ${entry.id} 不能更换内容；请发布新版本`);
    }
  }
  const verified = [];
  if (!options.offline) for (const entry of catalog.plugins) verified.push(await verify(entry, validate));
  if (options.check) {
    if (!(await readBounded(output, MAX_CATALOG)).equals(bytes)) throw new Error('目录与条目不一致');
  } else await writeAtomic(output, bytes);
  return { count: entries.length, bytes: bytes.length, output, checked: !!options.check, releasesVerified: !options.offline, verified };
}

export async function packPlugin(options, validate) {
  const info = await lstat(options.input);
  if (!info.isDirectory() || info.isSymbolicLink()) throw new Error('插件路径必须是普通目录');
  const directory = await realpath(options.input), rawManifest = await readBounded(path.join(directory, 'mewu-plugin.json'), 64 * 1024);
  const manifest = await validate('extension-manifest', rawManifest);
  const names = ['mewu-plugin.json', manifest.entry, ...(options.files ?? [])];
  if (new Set(names.map(name => name.toLowerCase())).size !== names.length) throw new Error('重复的插件包文件');
  const files = []; let size = 0;
  for (const name of names) {
    packagePath(name);
    const filename = path.join(directory, ...name.split('/'));
    let current = directory;
    for (const part of name.split('/').slice(0, -1)) {
      current = path.join(current, part);
      const parent = await lstat(current);
      if (!parent.isDirectory() || parent.isSymbolicLink()) throw new Error('插件目录不能通过链接读取文件');
    }
    const resolved = await realpath(filename);
    if (!within(directory, resolved)) throw new Error('插件文件离开包目录');
    const bytes = name === 'mewu-plugin.json' ? rawManifest : await readBounded(filename, 1024 * 1024);
    size += bytes.length; if (size > MAX_EXPANDED_BYTES) throw new Error('插件包展开体积超过限制');
    files.push({ name, bytes });
  }
  const bytes = createZip(files); readZip(bytes);
  const output = path.resolve(options.out);
  if (!output.endsWith('.zip') || within(directory, output)) throw new Error('输出必须是插件目录之外的 .zip 文件');
  await writeAtomic(output, bytes);
  return { id: manifest.id, version: manifest.version, output, size: bytes.length, sha256: sha256(bytes), files: files.length };
}

function ghUpload(repository, tag, filename) {
  return new Promise((resolve, reject) => {
    const child = spawn('gh', ['release', 'upload', tag, filename, '--repo', repository], { shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let count = 0, failed = false;
    const timer = setTimeout(() => { failed = true; child.kill('SIGKILL'); }, 180_000);
    const consume = chunk => { count += chunk.length; if (count > 64 * 1024) { failed = true; child.kill('SIGKILL'); } };
    child.stdout.on('data', consume); child.stderr.on('data', consume);
    child.on('error', () => { failed = true; });
    child.on('close', code => { clearTimeout(timer); if (failed || code !== 0) reject(new Error('GitHub 上传失败；未覆盖任何已有资产，请检查指定发行及 gh 登录')); else resolve(); });
  });
}

export async function publishPlugin(options, validate, network = { getRelease, downloadReleaseAsset, upload: ghUpload }) {
  const bytes = await readBounded(options.input, MAX_PACKAGE_BYTES), files = readZip(bytes);
  const manifest = await validate('extension-manifest', files.get('mewu-plugin.json'));
  if (!files.has(manifest.entry)) throw new Error('ZIP 缺少清单指定的 JavaScript 入口');
  const entry = { id: manifest.id, name: manifest.name, description: manifest.description, tags: manifest.tags,
    repository: options.repository, version: manifest.version, releaseTag: options.releaseTag, assetName: path.basename(options.input),
    sha256: sha256(bytes), size: bytes.length, hostApi: 2, platforms: options.platforms?.length ? options.platforms : ['any'] };
  const validated = await validate('extension-entry', Buffer.from(JSON.stringify(entry)));
  bindEntryManifest(validated, manifest);
  const output = path.resolve(options.entryOut);
  if (path.basename(output) !== `${entry.id}.json`) throw new Error('目录条目输出文件名必须为 id.json');
  await realpath(path.dirname(output));
  try {
    const previous = await validate('extension-entry', await readBounded(output, MAX_CATALOG));
    if (previous.id !== entry.id || previous.repository.toLowerCase() !== entry.repository.toLowerCase()) throw new Error('禁止把目录条目改到其他插件或 GitHub 来源');
    if (previous.version === entry.version && (previous.sha256 !== entry.sha256 || previous.size !== entry.size)) throw new Error('同一版本不能更换插件内容；请发布新版本');
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
  let release = await network.getRelease(entry.repository, entry.releaseTag);
  const existing = release.assets.filter(asset => asset.name === entry.assetName);
  if (existing.length > 1 || (existing.length && (existing[0].size !== entry.size || (existing[0].digest != null && existing[0].digest !== `sha256:${entry.sha256}`)))) {
    throw new Error('发行中存在同名不同内容的资产；禁止覆盖，必须发布新版本');
  }
  if (!existing.length) {
    await network.upload(entry.repository, entry.releaseTag, options.input);
    release = await network.getRelease(entry.repository, entry.releaseTag);
  }
  const downloaded = await network.downloadReleaseAsset(entry.repository, release, entry.assetName, entry);
  if (!downloaded.bytes.equals(bytes)) throw new Error('GitHub 发行的实际 ZIP 与本地包不一致');
  await writeAtomic(output, Buffer.from(`${JSON.stringify(validated, null, 2)}\n`));
  return { id: entry.id, version: entry.version, repository: entry.repository, releaseTag: entry.releaseTag, assetName: entry.assetName,
    sha256: entry.sha256, size: entry.size, entryOutput: output, uploaded: !existing.length, verified: true };
}

export async function run(args, io = { stdout: process.stdout, stderr: process.stderr }, dependencies = {}) {
  const json = args.includes('--json');
  try {
    const options = optionsFor(args);
    if (options.help) { io.stdout.write(HELP); return 0; }
    const validate = dependencies.validate ?? await createValidator(options.validator);
    let details;
    if (options.kind === 'build') details = await buildCatalog(options, validate, dependencies.verify);
    else if (options.kind === 'build-projects') details = await buildProjects(options, validate, dependencies.verifyProject);
    else if (['project', 'projects', 'verify-project'].includes(options.kind)) {
      const value = await validate(options.kind === 'projects' ? 'extension-projects' : 'extension-project', await readBounded(options.input, options.kind === 'projects' ? MAX_CATALOG : 64 * 1024));
      if (options.kind === 'verify-project') {
        const catalog = await validate('extension-catalog', await readBounded(options.catalog, MAX_CATALOG));
        const entry = catalog.plugins.find(entry => entry.id === value.id);
        bindProjectEntry(value, entry);
        details = { ...await (dependencies.verifyProject ?? verifyProject)(value, entry, validate), verified: true };
      } else details = options.kind === 'projects' ? { count: value.projects.length } : { id: value.id, version: value.version };
    }
    else if (options.kind === 'pack') details = await packPlugin(options, validate);
    else if (options.kind === 'publish') details = await publishPlugin(options, validate, dependencies.network);
    else {
      const mode = options.kind === 'verify' ? 'entry' : options.kind;
      const value = await validate(`extension-${mode}`, await readBounded(options.input, mode === 'manifest' ? 64 * 1024 : MAX_CATALOG));
      details = options.kind === 'verify' ? { ...await verifyEntry(value, validate), verified: true }
        : mode === 'catalog' ? { count: value.plugins.length } : { id: value.id, version: value.version };
    }
    const result = { ok: true, hostApi: 2, kind: options.kind, ...details };
    io.stdout.write(json ? `${JSON.stringify(result)}\n` : `完成：${details.id ?? `${details.count} 个插件`}${details.releasesVerified === false ? '（仅离线结构检查）' : ''}\n`);
    return 0;
  } catch (error) {
    const message = error instanceof Error ? error.message : '未知错误';
    (json ? io.stdout : io.stderr).write(json ? `${JSON.stringify({ ok: false, hostApi: 2, error: message })}\n` : `失败：${message}\n`);
    return 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) process.exitCode = await run(process.argv.slice(2));
