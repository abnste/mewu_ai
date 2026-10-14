// SPDX-License-Identifier: MPL-2.0
// Deliberately small ZIP profile: ordinary files, no encryption, ZIP64 or links.
import { inflateRawSync } from 'node:zlib';

export const MAX_PACKAGE_BYTES = 16 * 1024 * 1024;
export const MAX_EXPANDED_BYTES = 8 * 1024 * 1024;
export const MAX_PACKAGE_FILES = 129; // 128 modules plus the root manifest.
const utf8 = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
const crcTable = Array.from({ length: 256 }, (_, value) => {
  for (let bit = 0; bit < 8; bit++) value = (value & 1) ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
  return value >>> 0;
});
export function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

export function packagePath(name) {
  if (typeof name !== 'string' || name.length > 240 || !/^[A-Za-z0-9_.\-/]+$/.test(name)
      || name.startsWith('/') || name.endsWith('/') || name.split('/').some(part => !part || part === '.' || part === '..')) {
    throw new Error('插件包路径不安全或不是普通文件');
  }
  if (name.split('/').some(part => /^(?:\.git|\.private|node_modules|target|dist)$/i.test(part)
      || /^\.env(?:\.|$)/i.test(part) || /^(?:agent|agents)\.md$/i.test(part)
      || /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(part) || part.endsWith('.'))) {
    throw new Error('插件包不能包含私有文件、构建缓存或系统保留路径');
  }
  if (name !== 'mewu-plugin.json' && !/\.(?:js|mjs)$/.test(name)) {
    throw new Error('首版 JavaScript 插件包不支持此文件类型');
  }
  return name;
}

function validateContent(name, bytes) {
  const limit = name === 'mewu-plugin.json' ? 64 * 1024 : 1024 * 1024;
  if (bytes.length > limit) throw new Error('插件清单或 JavaScript 模块超过体积限制');
  const source = utf8.decode(bytes);
  if (source.startsWith('\ufeff') || source.includes('\0')) throw new Error('插件源码不能包含 BOM 或 NUL');
}

// Stored members and a fixed 1980 timestamp produce identical bytes on all OSes.
export function createZip(inputFiles) {
  const files = [...inputFiles].sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
  if (!files.length || files.length > MAX_PACKAGE_FILES) throw new Error('插件包文件数量无效');
  const seen = new Set(), local = [], central = [];
  let offset = 0, expanded = 0;
  for (const file of files) {
    packagePath(file.name);
    const key = file.name.toLowerCase();
    if (seen.has(key)) throw new Error('插件包存在重复或大小写冲突路径');
    seen.add(key);
    const bytes = Buffer.from(file.bytes), name = Buffer.from(file.name, 'utf8');
    validateContent(file.name, bytes);
    expanded += bytes.length;
    if (expanded > MAX_EXPANDED_BYTES) throw new Error('插件包展开体积过大');
    const crc = crc32(bytes);
    const header = Buffer.alloc(30);
    header.writeUInt32LE(0x04034b50, 0); header.writeUInt16LE(20, 4); header.writeUInt16LE(0x800, 6);
    header.writeUInt16LE(0x21, 12); header.writeUInt32LE(crc, 14);
    header.writeUInt32LE(bytes.length, 18); header.writeUInt32LE(bytes.length, 22); header.writeUInt16LE(name.length, 26);
    local.push(header, name, bytes);
    const record = Buffer.alloc(46);
    record.writeUInt32LE(0x02014b50, 0); record.writeUInt16LE(0x0314, 4); record.writeUInt16LE(20, 6);
    record.writeUInt16LE(0x800, 8); record.writeUInt16LE(0x21, 14); record.writeUInt32LE(crc, 16);
    record.writeUInt32LE(bytes.length, 20); record.writeUInt32LE(bytes.length, 24); record.writeUInt16LE(name.length, 28);
    record.writeUInt32LE((0o100644 * 65536) >>> 0, 38); record.writeUInt32LE(offset, 42);
    central.push(record, name); offset += header.length + name.length + bytes.length;
  }
  const directory = Buffer.concat(central), end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(files.length, 8); end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(directory.length, 12); end.writeUInt32LE(offset, 16);
  const archive = Buffer.concat([...local, directory, end]);
  if (archive.length > MAX_PACKAGE_BYTES) throw new Error('插件 ZIP 超过 16 MiB');
  return archive;
}

export function readZip(input) {
  const bytes = Buffer.from(input);
  if (bytes.length < 22 || bytes.length > MAX_PACKAGE_BYTES) throw new Error('插件 ZIP 体积无效');
  const end = bytes.length - 22;
  if (bytes.readUInt32LE(end) !== 0x06054b50 || bytes.readUInt16LE(end + 20) !== 0
      || bytes.readUInt16LE(end + 4) !== 0 || bytes.readUInt16LE(end + 6) !== 0) {
    throw new Error('仅支持无注释、单卷 ZIP 插件包');
  }
  const count = bytes.readUInt16LE(end + 10), directorySize = bytes.readUInt32LE(end + 12);
  const directoryOffset = bytes.readUInt32LE(end + 16);
  if (!count || count > MAX_PACKAGE_FILES || count !== bytes.readUInt16LE(end + 8)
      || directoryOffset + directorySize !== end) throw new Error('插件 ZIP 中央目录无效');
  const files = new Map(), names = new Set(), ranges = [];
  let at = directoryOffset, expanded = 0;
  for (let index = 0; index < count; index++) {
    if (at + 46 > end || bytes.readUInt32LE(at) !== 0x02014b50) throw new Error('插件 ZIP 条目不完整');
    const flags = bytes.readUInt16LE(at + 8), method = bytes.readUInt16LE(at + 10), crc = bytes.readUInt32LE(at + 16);
    const compressedSize = bytes.readUInt32LE(at + 20), size = bytes.readUInt32LE(at + 24);
    const nameSize = bytes.readUInt16LE(at + 28), extraSize = bytes.readUInt16LE(at + 30), commentSize = bytes.readUInt16LE(at + 32);
    const attributes = bytes.readUInt32LE(at + 38), mode = attributes >>> 16, localOffset = bytes.readUInt32LE(at + 42);
    if ((flags & ~0x800) !== 0 || ![0, 8].includes(method) || extraSize || commentSize
        || bytes.readUInt16LE(at + 34) || (attributes & 0x10) || (mode && (mode & 0o170000) !== 0o100000)
        || at + 46 + nameSize > end || size > MAX_EXPANDED_BYTES) throw new Error('插件 ZIP 不支持加密、链接、特殊文件或扩展字段');
    const nameBytes = bytes.subarray(at + 46, at + 46 + nameSize), name = packagePath(utf8.decode(nameBytes));
    if (names.has(name.toLowerCase())) throw new Error('插件 ZIP 存在重复或大小写冲突路径');
    names.add(name.toLowerCase());
    if (localOffset + 30 > directoryOffset || bytes.readUInt32LE(localOffset) !== 0x04034b50) throw new Error('插件 ZIP 本地条目无效');
    const localNameSize = bytes.readUInt16LE(localOffset + 26), localExtraSize = bytes.readUInt16LE(localOffset + 28);
    const dataOffset = localOffset + 30 + localNameSize + localExtraSize, dataEnd = dataOffset + compressedSize;
    if (dataEnd > directoryOffset || localExtraSize || flags !== bytes.readUInt16LE(localOffset + 6)
        || method !== bytes.readUInt16LE(localOffset + 8) || crc !== bytes.readUInt32LE(localOffset + 14)
        || compressedSize !== bytes.readUInt32LE(localOffset + 18) || size !== bytes.readUInt32LE(localOffset + 22)
        || !nameBytes.equals(bytes.subarray(localOffset + 30, localOffset + 30 + localNameSize))) {
      throw new Error('插件 ZIP 本地与中央目录不一致');
    }
    expanded += size;
    if (expanded > MAX_EXPANDED_BYTES) throw new Error('插件 ZIP 展开体积过大');
    const compressed = bytes.subarray(dataOffset, dataEnd);
    const content = method === 0 ? compressed : inflateRawSync(compressed, { maxOutputLength: Math.max(size, 1) });
    if (content.length !== size || crc32(content) !== crc) throw new Error('插件 ZIP 文件体积或 CRC32 不一致');
    validateContent(name, content);
    files.set(name, Buffer.from(content)); ranges.push([localOffset, dataEnd]); at += 46 + nameSize;
  }
  if (at !== end) throw new Error('插件 ZIP 中央目录存在未声明数据');
  ranges.sort((a, b) => a[0] - b[0]);
  if (ranges[0][0] !== 0 || ranges.some((range, index) => range[1] !== (ranges[index + 1]?.[0] ?? directoryOffset))) {
    throw new Error('插件 ZIP 存在重叠、前缀或未声明文件');
  }
  if (!files.has('mewu-plugin.json')) throw new Error('插件包根目录缺少 mewu-plugin.json');
  return files;
}
