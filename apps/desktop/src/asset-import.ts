// SPDX-License-Identifier: MPL-2.0
import type { Asset, AssetKind } from './contracts';

export const IMPORT_FILE_LIMIT = 12;
export const IMPORT_BATCH_BYTES = 128 * 1024 * 1024;
export const IMPORT_IMAGE_BYTES = 32 * 1024 * 1024;
export const IMPORT_DOCUMENT_BYTES = 2 * 1024 * 1024;
export const TEXT_PAGE_BYTES = 64 * 1024;
export const IMPORT_ACCEPT = '.png,.jpg,.jpeg,.webp,.html,.htm,.svg,.txt,.md,.json,.csv';

export function importedKind(name: string): AssetKind {
  const extension = name.toLowerCase().split('.').pop();
  if (['png', 'jpg', 'jpeg', 'webp'].includes(extension ?? '')) return 'image';
  if (extension === 'html' || extension === 'htm') return 'html';
  if (extension === 'svg') return 'svg';
  if (['txt', 'md', 'json', 'csv'].includes(extension ?? '')) return 'text';
  throw new Error('不支持此文件格式');
}

export function decodeImportedText(bytes: ArrayBuffer, kind: AssetKind): string {
  if (bytes.byteLength > IMPORT_DOCUMENT_BYTES) throw new Error('文本文件超过 2 MiB');
  let text: string;
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
  catch { throw new Error('文件不是有效的 UTF-8 文本'); }
  if (kind === 'text' && text.includes('\0')) throw new Error('文本文件含有 NUL 字符');
  return text;
}

export interface PreparedImport { asset: Asset; text?: string }
interface ImportEnvironment {
  id(): string;
  imageSize(file: File): Promise<{ width: number; height: number }>;
  createUrl(file: File): string;
  revokeUrl(url: string): void;
}

/** Validate the entire batch before the caller publishes any scene changes. */
export async function prepareImportFiles(files: File[], environment: ImportEnvironment): Promise<PreparedImport[]> {
  if (files.length > IMPORT_FILE_LIMIT) throw new Error('一次最多导入 12 个文件');
  let total = 0;
  const kinds = files.map(file => {
    const kind = importedKind(file.name);
    if (!Number.isSafeInteger(file.size) || file.size < 0) throw new Error('文件大小无效');
    if (kind === 'image' && file.size === 0) throw new Error('无法打开图片');
    if (file.size > (kind === 'image' ? IMPORT_IMAGE_BYTES : IMPORT_DOCUMENT_BYTES)) throw new Error(kind === 'image' ? '图片超过 32 MiB' : '文本文件超过 2 MiB');
    total += file.size;
    if (total > IMPORT_BATCH_BYTES) throw new Error('本次文件合计超过 128 MiB');
    return kind;
  });
  const prepared: PreparedImport[] = [];
  try {
    for (let index = 0; index < files.length; index++) {
      const file = files[index], kind = kinds[index];
      let text: string | undefined, dimensions: { width: number; height: number } | undefined;
      if (kind === 'image') {
        dimensions = await environment.imageSize(file);
        if (!Number.isSafeInteger(dimensions.width) || !Number.isSafeInteger(dimensions.height) || dimensions.width <= 0 || dimensions.height <= 0 || dimensions.width * dimensions.height > 40_000_000) throw new Error('图片尺寸超出限制');
      } else text = decodeImportedText(await file.arrayBuffer(), kind);
      prepared.push({ asset: { id: environment.id(), name: file.name, kind, path: environment.createUrl(file), ...dimensions }, text });
    }
    return prepared;
  } catch (error) {
    for (const value of prepared) environment.revokeUrl(value.asset.path);
    throw error;
  }
}

/** UTF-8 byte-bounded pages without splitting surrogate pairs or altering text. */
export function textPageBoundaries(text: string): number[] {
  const boundaries = [0];
  let bytes = 0;
  for (let index = 0; index < text.length;) {
    const code = text.codePointAt(index)!;
    const size = code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
    if (bytes + size > TEXT_PAGE_BYTES) { boundaries.push(index); bytes = 0; }
    bytes += size; index += code > 0xffff ? 2 : 1;
  }
  if (boundaries[boundaries.length - 1] !== text.length) boundaries.push(text.length);
  if (boundaries.length === 1) boundaries.push(0);
  return boundaries;
}
