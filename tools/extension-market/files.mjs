// SPDX-License-Identifier: MPL-2.0
import { constants } from 'node:fs';
import { lstat, open, realpath, rename, unlink } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
class ToolError extends Error {}

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

