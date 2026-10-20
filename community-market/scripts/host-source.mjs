// SPDX-License-Identifier: MPL-2.0
import { appendFile, readFile } from 'node:fs/promises';
const config = JSON.parse(await readFile(new URL('../host-source.json', import.meta.url), 'utf8'));
if (Object.keys(config).sort().join(',') !== 'commit,repository' || config.repository !== 'abnste/mewu_ai'
    || typeof config.commit !== 'string' || !/^[a-f0-9]{40}$/.test(config.commit)) {
  throw new Error('维护者须先将 host-source.json 固定到已发布 API 校验器源码的完整提交；不得使用 next/main 或 PR 提供的源码');
}
if (!process.env.GITHUB_OUTPUT) throw new Error('仅供 GitHub Actions 输出固定可信宿主源码');
await appendFile(process.env.GITHUB_OUTPUT, `repository=${config.repository}\ncommit=${config.commit}\n`, 'utf8');
