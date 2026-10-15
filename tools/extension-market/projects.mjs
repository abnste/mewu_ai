// SPDX-License-Identifier: MPL-2.0
// Project metadata is parsed by the production Rust contract, never a JS schema.
import { createHash } from 'node:crypto';
import { lstat, readdir, realpath } from 'node:fs/promises';
import path from 'node:path';
import { readBounded, writeAtomic } from '../plugin-catalog/cli.mjs';
import { bindEntryManifest, downloadReleaseAsset, getRelease, githubBytes, repositoryParts, sha256 } from './github.mjs';
import { crc32, readZip } from './zip.mjs';

const MAX_PROJECTS = 2 * 1024 * 1024;
const utf8 = new TextDecoder('utf-8', { fatal: true });
const objectHash = bytes => createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex');
const sourceHash = /^[a-f0-9]{40}$/;

// GitHub tree/blob identities bind every byte to an immutable commit. Symlinks
// and submodules are not file sources, even when the URL would follow them.
export async function sourceReader(repository, commit, options = {}) {
  const repo = repositoryParts(repository);
  if (!sourceHash.test(commit)) throw new Error('项目必须固定完整 Git 提交');
  const get = async endpoint => JSON.parse(utf8.decode(await githubBytes(`https://api.github.com/repos/${repo}${endpoint}`, options)));
  const revision = await get(`/git/commits/${commit}`);
  if (revision.sha !== commit || !sourceHash.test(revision.tree?.sha ?? '')) throw new Error('GitHub 项目提交身份不符');
  const tree = await get(`/git/trees/${revision.tree.sha}?recursive=1`);
  if (tree.sha !== revision.tree.sha || tree.truncated !== false || !Array.isArray(tree.tree) || tree.tree.length > 4096) throw new Error('项目 Git tree 不完整或过大');
  const nodes = new Map();
  for (const node of tree.tree) {
    if (typeof node.path !== 'string' || nodes.has(node.path)) throw new Error('项目 Git tree 路径重复');
    nodes.set(node.path, node);
  }
  return async (name, limit = 1024 * 1024) => {
    const node = nodes.get(name);
    if (!node || node.type !== 'blob' || !['100644', '100755'].includes(node.mode)
        || !sourceHash.test(node.sha ?? '') || !Number.isSafeInteger(node.size) || node.size < 0 || node.size > limit) throw new Error(`项目缺少普通文件：${name}`);
    // GitHub wraps base64 at 60 columns; JSON encodes each newline as two
    // bytes. Keep that bounded overhead instead of rejecting near-limit files.
    const raw = await githubBytes(`https://api.github.com/repos/${repo}/git/blobs/${node.sha}`, { ...options, limit: Math.ceil(limit * 1.5) + 16384 });
    const blob = JSON.parse(utf8.decode(raw));
    if (blob.sha !== node.sha || blob.encoding !== 'base64' || typeof blob.content !== 'string') throw new Error('项目 Git blob 身份不符');
    const bytes = Buffer.from(blob.content, 'base64');
    if (bytes.length !== node.size || objectHash(bytes) !== node.sha) throw new Error('项目源码字节与固定 Git tree 不符');
    return bytes;
  };
}

export function checkPngCrc(bytes) {
  if (!bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return;
  let at = 8, ended = false, count = 0;
  while (at < bytes.length) {
    if (++count > 4096 || at + 12 > bytes.length) throw new Error('PNG 块截断或过多');
    const size = bytes.readUInt32BE(at), end = at + 12 + size;
    if (end > bytes.length) throw new Error('PNG 块截断');
    if (crc32(bytes.subarray(at + 4, end - 4)) !== bytes.readUInt32BE(end - 4)) throw new Error('PNG CRC32 不符');
    const kind = bytes.toString('ascii', at + 4, at + 8);
    if (count === 1 && (kind !== 'IHDR' || size !== 13)) throw new Error('PNG 缺少首个 IHDR');
    if (kind === 'IEND') { if (size !== 0 || end !== bytes.length) throw new Error('PNG 结束块无效'); ended = true; }
    at = end;
  }
  if (!ended) throw new Error('PNG 缺少 IEND');
}

export function bindProjectEntry(project, entry) {
  if (!entry || project.id !== entry.id || project.version !== entry.version || project.packageSha256 !== entry.sha256) throw new Error('项目与实际目录版本或安装包 SHA256 不符');
}

export async function verifyProject(project, entry, validate, options = {}) {
  bindProjectEntry(project, entry);
  const release = await getRelease(entry.repository, entry.releaseTag, options);
  const downloaded = await downloadReleaseAsset(entry.repository, release, entry.assetName, entry, options);
  const files = readZip(downloaded.bytes), manifest = await validate('extension-manifest', files.get('mewu-plugin.json'));
  bindEntryManifest(entry, manifest);
  if (!files.has(manifest.entry)) throw new Error('发行包缺少入口');
  const source = await sourceReader(project.sourceRepository, project.sourceCommit, options);
  const license = await source('LICENSE', 128 * 1024);
  if (!license.length) throw new Error('独立项目缺少许可文本');
  const modules = [];
  for (const [name, bytes] of files) {
    if (name !== 'mewu-plugin.json' && !/\.(?:js|mjs)$/.test(name) && !['LICENSE', 'README.md'].includes(name)) continue;
    const remote = await source(name, name === 'mewu-plugin.json' ? 64 * 1024 : 1024 * 1024);
    if (!remote.equals(bytes)) throw new Error(`项目源码与真实发行 ZIP 不同：${name}`);
    modules.push({ path: name, bytes: bytes.length, sha256: sha256(bytes) });
  }
  const screenshots = [];
  for (let index = 0; index < project.screenshots.length; index++) {
    const shot = project.screenshots[index], bytes = await source(shot.path, 2 * 1024 * 1024);
    if (bytes.length !== shot.size || sha256(bytes) !== shot.sha256) throw new Error('项目截图字节或 SHA256 不符');
    checkPngCrc(bytes);
    // Actual decode and size/dimension limits come from the same image loader
    // used by the application; no browser execution or author scripts involved.
    await validate('extension-project-image', Buffer.from(JSON.stringify({ project, index, data: bytes.toString('base64') })));
    screenshots.push({ path: shot.path, size: bytes.length, sha256: shot.sha256, width: shot.width, height: shot.height });
  }
  return { id: project.id, version: project.version, packageSha256: downloaded.sha256, sourceRepository: project.sourceRepository,
    sourceCommit: project.sourceCommit, licenseSha256: sha256(license), modules, screenshots, sourceAndPackageBytesMatched: true };
}

export async function buildProjects(options, validate, verify = verifyProject) {
  const info = await lstat(options.input);
  if (!info.isDirectory() || info.isSymbolicLink()) throw new Error('项目条目必须是普通目录');
  const directory = await realpath(options.input), output = path.join(await realpath(path.dirname(options.out)), path.basename(options.out));
  const relative = path.relative(directory, output);
  if (relative === '' || !relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative)) throw new Error('输出不能在 projects 条目目录内');
  const catalog = await validate('extension-catalog', await readBounded(options.catalog, MAX_PROJECTS));
  const files = await readdir(directory, { withFileTypes: true });
  if (files.length > 2048) throw new Error('项目数量超过限制');
  const projects = []; let total = 0;
  for (const file of files.sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
    if (!file.isFile() || file.isSymbolicLink() || !file.name.endsWith('.json')) throw new Error('projects 只能包含直属 JSON 普通文件');
    const bytes = await readBounded(path.join(directory, file.name), 64 * 1024);
    total += bytes.length; if (total > MAX_PROJECTS) throw new Error('项目元数据超过 2 MiB');
    const project = await validate('extension-project', bytes);
    if (file.name !== `${project.id}.json`) throw new Error('项目文件名必须是插件 id.json');
    bindProjectEntry(project, catalog.plugins.find(entry => entry.id === project.id));
    projects.push(project);
  }
  projects.sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  if (catalog.plugins.some(entry => !projects.some(project => project.id === entry.id))) throw new Error('每个社区插件必须有匹配的独立源码项目');
  const bytes = Buffer.from(`${JSON.stringify({ schemaVersion: 1, projects }, null, 2)}\n`);
  if (bytes.length > MAX_PROJECTS) throw new Error('项目目录超过 2 MiB');
  await validate('extension-projects', bytes);
  let previous;
  try { previous = await validate('extension-projects', await readBounded(options.previous ?? output, MAX_PROJECTS)); }
  catch (error) { if (error.code !== 'ENOENT') throw error; if (options.previous) throw new Error('指定的可信旧项目目录不存在'); }
  const prior = new Map(previous?.projects.map(project => [project.id, project]) ?? []);
  for (const project of projects) if (prior.has(project.id) && prior.get(project.id).sourceRepository.toLowerCase() !== project.sourceRepository.toLowerCase()) throw new Error('已有插件不能更换独立源码项目');
  const verified = [];
  if (!options.offline) for (const project of projects) verified.push(await verify(project, catalog.plugins.find(entry => entry.id === project.id), validate));
  if (options.check) { if (!(await readBounded(output, MAX_PROJECTS)).equals(bytes)) throw new Error('项目目录与条目不一致'); }
  else await writeAtomic(output, bytes);
  return { count: projects.length, bytes: bytes.length, output, checked: !!options.check, projectsVerified: !options.offline, verified };
}
