// SPDX-License-Identifier: MPL-2.0
// Runs from trusted Mewu source. PR code is never checked out or evaluated.
import { createHash } from 'node:crypto';
import { lstat, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { readBounded, writeAtomic } from './files.mjs';
import { githubBytes } from './github.mjs';

const utf8 = new TextDecoder('utf-8', { fatal: true });
const repositoryPattern = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const commitPattern = /^[a-f0-9]{40}$/;
async function githubJson(endpoint, options) { return JSON.parse(utf8.decode(await githubBytes(`https://api.github.com${endpoint}`, options))); }

export async function collectPullRequest({ event, repository, output, projectOutput }, options = {}) {
  if (!repositoryPattern.test(repository)) throw new Error('社区仓库身份无效');
  const number = event.number;
  if (!Number.isSafeInteger(number) || number < 1 || event.repository?.full_name?.toLowerCase() !== repository.toLowerCase()) throw new Error('PR 事件身份无效');
  const pr = await githubJson(`/repos/${repository}/pulls/${number}`, options);
  if (pr.base?.repo?.full_name?.toLowerCase() !== repository.toLowerCase() || pr.state !== 'open'
      || !commitPattern.test(pr.head?.sha ?? '') || !repositoryPattern.test(pr.head?.repo?.full_name ?? '')
      || pr.head.sha !== event.pull_request?.head?.sha) throw new Error('PR 已改变或不属于此社区仓库');
  const changed = await githubJson(`/repos/${repository}/pulls/${number}/files?per_page=100`, options);
  const allowed = projectOutput ? /^(?:entries|projects)\/[A-Za-z0-9_.-]+\.json$/ : /^entries\/[A-Za-z0-9_.-]+\.json$/;
  if (!Array.isArray(changed) || !changed.length || changed.length > (projectOutput ? 6 : 3) || pr.changed_files !== changed.length
      || changed.some(file => !allowed.test(file.filename) || !['added', 'modified', 'removed'].includes(file.status))
      || new Set(changed.map(file => path.posix.basename(file.filename))).size > 3) throw new Error('插件收录 PR 只能修改最多三个插件的 entries/projects JSON；工具、流程与文档修改请另开维护 PR');
  const headRepository = pr.head.repo.full_name, tree = await githubJson(`/repos/${headRepository}/git/trees/${pr.head.sha}?recursive=1`, options);
  if (tree.truncated !== false || !Array.isArray(tree.tree) || tree.tree.length > 4096) throw new Error('PR Git tree 过大或不完整');
  const files = tree.tree.filter(file => file.path.startsWith('entries/') || projectOutput && file.path.startsWith('projects/'));
  if (files.length > (projectOutput ? 2304 : 256) || files.filter(file => file.path.startsWith('entries/')).length > 256
      || files.filter(file => file.path.startsWith('projects/')).length > 2048 || files.some(file => !allowed.test(file.path) || file.type !== 'blob'
      || file.mode !== '100644' || !commitPattern.test(file.sha) || !Number.isSafeInteger(file.size) || file.size > 64 * 1024)) {
    throw new Error('PR entries 含目录、链接、异常文件或过大条目');
  }
  try { await lstat(output); throw new Error('PR 输出目录必须尚未存在'); }
  catch (error) { if (error.code !== 'ENOENT') throw error; }
  await mkdir(output);
  if (projectOutput) {
    if (path.resolve(projectOutput) === path.resolve(output)) throw new Error('项目与安装条目输出必须分开');
    await mkdir(projectOutput); // Existing directories fail; never mix trusted and PR data.
  }
  let total = 0, projectBytes = 0, entryBytes = 0;
  for (const file of files) {
    const blob = await githubJson(`/repos/${headRepository}/git/blobs/${file.sha}`, options);
    if (blob.sha !== file.sha || blob.encoding !== 'base64' || typeof blob.content !== 'string' || blob.content.length > 100_000) throw new Error('GitHub PR blob 身份无效');
    const bytes = Buffer.from(blob.content, 'base64');
    const digest = createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex');
    if (bytes.length !== file.size || digest !== file.sha) throw new Error('GitHub PR blob 内容与固定 Git tree 不符');
    const project = file.path.startsWith('projects/');
    total += bytes.length; project ? projectBytes += bytes.length : entryBytes += bytes.length;
    if (entryBytes > 2 * 1024 * 1024 || projectBytes > 2 * 1024 * 1024) throw new Error('PR 单个目录总计超过 2 MiB');
    await writeAtomic(path.join(project ? projectOutput : output, path.basename(file.path)), bytes);
  }
  return { repository, number, head: pr.head.sha, entries: files.filter(file => file.path.startsWith('entries/')).length,
    ...(projectOutput ? { projects: files.filter(file => file.path.startsWith('projects/')).length } : {}), bytes: total };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    if (![3, 4].includes(process.argv.length) || !process.env.GITHUB_EVENT_PATH || !process.env.GITHUB_REPOSITORY) throw new Error('用法：collect-pr.mjs <未存在的条目目录> [未存在的项目目录]；仅供可信 GitHub Actions 使用');
    const event = JSON.parse(utf8.decode(await readBounded(process.env.GITHUB_EVENT_PATH, 2 * 1024 * 1024)));
    process.stdout.write(`${JSON.stringify(await collectPullRequest({ event, repository: process.env.GITHUB_REPOSITORY, output: path.resolve(process.argv[2]), ...(process.argv[3] ? { projectOutput: path.resolve(process.argv[3]) } : {}) }))}\n`);
  } catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 1; }
}
