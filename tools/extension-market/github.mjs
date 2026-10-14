// SPDX-License-Identifier: MPL-2.0
import { createHash } from 'node:crypto';
import { MAX_PACKAGE_BYTES, readZip } from './zip.mjs';

const apiOrigin = 'https://api.github.com';
const downloadHosts = new Set(['api.github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com']);
const utf8 = new TextDecoder('utf-8', { fatal: true });
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

function repositoryParts(repository) {
  if (typeof repository !== 'string' || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)
      || repository.split('/').some(part => part === '.' || part === '..' || part.endsWith('.git'))) {
    throw new Error('必须指定固定 GitHub owner/repository，不能使用任意 URL');
  }
  return repository.split('/').map(encodeURIComponent).join('/');
}

async function responseBytes(response, limit) {
  const length = response.headers.get('content-length');
  if (length && (!/^\d+$/.test(length) || Number(length) > limit)) throw new Error('GitHub 响应体积超过限制');
  const chunks = []; let size = 0;
  if (!response.body) throw new Error('GitHub 返回空响应体');
  try {
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > limit) throw new Error('GitHub 响应体积超过限制');
      chunks.push(Buffer.from(chunk));
    }
  } catch (error) { await response.body.cancel().catch(() => {}); throw error; }
  return Buffer.concat(chunks);
}

export async function githubBytes(url, { binary = false, limit = 2 * 1024 * 1024, fetchImpl = fetch } = {}) {
  let current = new URL(url);
  if (current.origin !== apiOrigin || current.username || current.password) throw new Error('请求必须从 GitHub API 开始');
  const controller = new AbortController(), timeout = setTimeout(() => controller.abort(), binary ? 120_000 : 30_000);
  try {
    for (let redirect = 0; redirect <= 4; redirect++) {
      if (current.protocol !== 'https:' || current.username || current.password || current.port
          || !downloadHosts.has(current.hostname) || (!binary && current.origin !== apiOrigin)) {
        throw new Error('GitHub 下载重定向离开允许来源');
      }
      const headers = { 'User-Agent': 'Mewu-Extension-Market/2', Accept: binary ? 'application/octet-stream' : 'application/vnd.github+json' };
      if (current.origin === apiOrigin) {
        headers['X-GitHub-Api-Version'] = '2022-11-28';
        // Credentials never follow asset redirects and are never included in errors.
        const token = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN;
        if (token) headers.Authorization = `Bearer ${token}`;
      }
      const response = await fetchImpl(current, { headers, redirect: 'manual', signal: controller.signal });
      if ([301, 302, 303, 307, 308].includes(response.status)) {
        const location = response.headers.get('location');
        await response.body?.cancel().catch(() => {});
        if (!location || redirect === 4) throw new Error('GitHub 下载重定向无效或过多');
        current = new URL(location, current); continue;
      }
      if (response.status !== 200) { await response.body?.cancel().catch(() => {}); throw new Error(`GitHub 请求失败（HTTP ${response.status}）`); }
      return await responseBytes(response, limit);
    }
    throw new Error('GitHub 下载未完成');
  } finally { clearTimeout(timeout); }
}

export async function getRelease(repository, releaseTag, options = {}) {
  const repo = repositoryParts(repository);
  const bytes = await githubBytes(`${apiOrigin}/repos/${repo}/releases/tags/${encodeURIComponent(releaseTag)}`, options);
  const release = JSON.parse(utf8.decode(bytes));
  if (release.tag_name !== releaseTag || release.draft !== false || !Number.isSafeInteger(release.id) || release.id <= 0
      || !Array.isArray(release.assets) || release.assets.length > 256) throw new Error('GitHub 返回的发行版本或资产列表无效');
  return release;
}

export async function downloadReleaseAsset(repository, release, assetName, expected = {}, options = {}) {
  const repo = repositoryParts(repository), matching = release.assets.filter(asset => asset.name === assetName);
  if (matching.length !== 1) throw new Error('GitHub 发行必须包含唯一的指定 ZIP 资产');
  const asset = matching[0];
  if (!Number.isSafeInteger(asset.id) || asset.id <= 0 || asset.state !== 'uploaded'
      || !Number.isSafeInteger(asset.size) || asset.size <= 0 || asset.size > MAX_PACKAGE_BYTES
      || asset.url !== `${apiOrigin}/repos/${repo}/releases/assets/${asset.id}`
      || asset.browser_download_url !== `https://github.com/${repo}/releases/download/${encodeURIComponent(release.tag_name)}/${encodeURIComponent(assetName)}`
      || (expected.size !== undefined && asset.size !== expected.size)) throw new Error('GitHub 资产身份、来源或体积不符');
  const bytes = await githubBytes(asset.url, { ...options, binary: true, limit: MAX_PACKAGE_BYTES });
  const digest = sha256(bytes);
  if (bytes.length !== asset.size || (expected.sha256 !== undefined && digest !== expected.sha256)
      || (asset.digest != null && asset.digest !== `sha256:${digest}`)) throw new Error('GitHub 插件资产 SHA256 或体积不符');
  return { bytes, sha256: digest, size: bytes.length, assetId: asset.id };
}

export function bindEntryManifest(entry, manifest) {
  for (const key of ['id', 'name', 'description', 'version', 'hostApi']) {
    if (entry[key] !== manifest[key]) throw new Error(`目录 ${key} 与实际插件清单不符`);
  }
  if (JSON.stringify([...entry.tags].sort()) !== JSON.stringify([...manifest.tags].sort())) throw new Error('目录标签与实际插件清单不符');
}

export async function verifyEntry(entry, validate, options = {}) {
  const release = await getRelease(entry.repository, entry.releaseTag, options);
  const downloaded = await downloadReleaseAsset(entry.repository, release, entry.assetName, entry, options);
  const files = readZip(downloaded.bytes), manifest = await validate('extension-manifest', files.get('mewu-plugin.json'));
  bindEntryManifest(entry, manifest);
  if (!files.has(manifest.entry)) throw new Error('插件 ZIP 缺少清单指定的 JavaScript 入口');
  return { id: entry.id, version: entry.version, repository: entry.repository, releaseTag: entry.releaseTag,
    assetName: entry.assetName, assetId: downloaded.assetId, sha256: downloaded.sha256, size: downloaded.size, files: files.size };
}
