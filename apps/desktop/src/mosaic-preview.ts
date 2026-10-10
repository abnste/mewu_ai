// SPDX-License-Identifier: MPL-2.0
export interface MosaicPreview { backgroundId: string; width: number; height: number; blockSize: number; columns: number; rows: number; dataUrl: string }

// In-flight work shares an exact CAS target; successful grids share immutable source pixels.
export class MosaicPreviewCache {
  private entries = new Map<string, { promise: Promise<MosaicPreview>; bytes: number; settled: boolean }>();
  private running = 0;
  private queue: (() => void)[] = [];
  constructor(private capacity = 16, private concurrency = 2, private maxBytes = 16 * 1024 * 1024) {}
  get(backgroundId: string, blockSize: number, load: () => Promise<MosaicPreview>, requestIdentity = ''): Promise<MosaicPreview> {
    const sourceKey = JSON.stringify(['source', backgroundId, blockSize]), key = JSON.stringify(['pending', backgroundId, blockSize, requestIdentity]);
    const hitKey = this.entries.has(sourceKey) ? sourceKey : key, existing = this.entries.get(hitKey);
    if (existing) { this.entries.delete(hitKey); this.entries.set(hitKey, existing); return existing.promise; }
    if (this.entries.size >= this.capacity) {
      const oldest = [...this.entries].find(([, value]) => value.settled);
      if (oldest) this.entries.delete(oldest[0]); else return Promise.reject(new Error('马赛克预览繁忙，请重试'));
    }
    const request = new Promise<MosaicPreview>((resolve, reject) => {
      const start = () => {
        this.running++;
        Promise.resolve().then(load).then(resolve, reject).finally(() => { this.running--; this.queue.shift()?.(); });
      };
      if (this.running < this.concurrency) start(); else this.queue.push(start);
    });
    const entry = { promise: request, bytes: 0, settled: false }; this.entries.set(key, entry);
    void request.then(value => {
      if (this.entries.get(key) !== entry) return;
      this.entries.delete(key);
      entry.settled = true; entry.bytes = value.dataUrl.length;
      this.entries.delete(sourceKey); this.entries.set(sourceKey, entry);
      let bytes = [...this.entries.values()].reduce((sum, value) => sum + value.bytes, 0);
      for (const [key, value] of this.entries) {
        if (bytes <= this.maxBytes) break;
        if (value.settled) { bytes -= value.bytes; this.entries.delete(key); }
      }
    }, () => { if (this.entries.get(key) === entry) this.entries.delete(key); });
    return request;
  }
}

export function mosaicBlockSize(scaleFactor?: number) { return Math.max(6, Math.min(40, Math.round(12 * (scaleFactor && Number.isFinite(scaleFactor) && scaleFactor > 0 ? scaleFactor : 1)))); }

// Average complete source-aligned blocks; partial edge blocks use only existing pixels.
export function mosaicGrid(rgba: Uint8ClampedArray, width: number, height: number, blockSize: number) {
  if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1 || width > 16384 || height > 16384 || width * height > 32 * 1024 * 1024 || !Number.isInteger(blockSize) || blockSize < 6 || blockSize > 40 || rgba.length !== width * height * 4) throw new Error('马赛克图像参数无效');
  const columns = Math.ceil(width / blockSize), rows = Math.ceil(height / blockSize);
  const data = new Uint8ClampedArray(columns * rows * 4);
  for (let row = 0; row < rows; row++) for (let column = 0; column < columns; column++) {
    const xEnd = Math.min(width, (column + 1) * blockSize), yEnd = Math.min(height, (row + 1) * blockSize);
    let red = 0, green = 0, blue = 0, count = 0;
    for (let y = row * blockSize; y < yEnd; y++) for (let x = column * blockSize; x < xEnd; x++) {
      const offset = (y * width + x) * 4, alpha = rgba[offset + 3];
      const white = 255 * (255 - alpha) + 127;
      red += Math.floor((rgba[offset] * alpha + white) / 255); green += Math.floor((rgba[offset + 1] * alpha + white) / 255); blue += Math.floor((rgba[offset + 2] * alpha + white) / 255); count++;
    }
    const offset = (row * columns + column) * 4;
    data[offset] = Math.floor(red / count); data[offset + 1] = Math.floor(green / count); data[offset + 2] = Math.floor(blue / count); data[offset + 3] = 255;
  }
  return { columns, rows, data };
}
export async function browserMosaicPreview(backgroundId: string, url: string, blockSize: number): Promise<MosaicPreview> {
  const image = new Image(); image.crossOrigin = 'anonymous'; image.src = url; await image.decode();
  const width = image.naturalWidth, height = image.naturalHeight;
  if (!width || !height || width > 16384 || height > 16384 || width * height > 32 * 1024 * 1024 || !Number.isInteger(blockSize) || blockSize < 6 || blockSize > 40) throw new Error('截图尺寸无法生成预览');
  const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
  const context = canvas.getContext('2d', { willReadFrequently: true }); if (!context) throw new Error('无法生成马赛克预览');
  context.drawImage(image, 0, 0);
  const grid = mosaicGrid(context.getImageData(0, 0, width, height).data, width, height, blockSize);
  canvas.width = grid.columns; canvas.height = grid.rows;
  const small = context.createImageData(grid.columns, grid.rows); small.data.set(grid.data); context.putImageData(small, 0, 0);
  return { backgroundId, width, height, blockSize, columns: grid.columns, rows: grid.rows, dataUrl: canvas.toDataURL('image/png') };
}
