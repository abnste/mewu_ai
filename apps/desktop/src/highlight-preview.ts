// SPDX-License-Identifier: MPL-2.0
export interface HighlightPreview { backgroundId: string; x: number; y: number; width: number; height: number; dataUrl: string }
// Exclude solid foreground and nonopaque originals; every partial-coverage
// pixel still gets its recovered foreground composed over the marker tint.
export async function browserHighlightPreview(backgroundId: string, url: string, crop?: { x: number; y: number; width: number; height: number }): Promise<HighlightPreview> {
  const image = new Image(); image.crossOrigin = 'anonymous'; image.src = url; await image.decode();
  const { x, y, width, height } = crop ?? { x: 0, y: 0, width: image.naturalWidth, height: image.naturalHeight };
  if (![x, y, width, height].every(Number.isSafeInteger) || x < 0 || y < 0 || x + width > image.naturalWidth || y + height > image.naturalHeight) throw Error('荧光笔选区已失效');
  if (width < 1 || height < 1 || width > 16384 || height > 16384 || width * height > 32 * 1024 * 1024) throw Error('荧光笔图片尺寸无效');
  const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
  const context = canvas.getContext('2d', { willReadFrequently: true }); if (!context) throw Error('无法生成荧光笔预览');
  try {
    context.drawImage(image, x, y, width, height, 0, 0, width, height);
    const input = context.getImageData(0, 0, width, height);
    const worker = new Worker(new URL('./highlight.worker.ts', import.meta.url), { type: 'module' });
    const data = await new Promise<Uint8ClampedArray>((resolve, reject) => {
      const finish = (value?: Uint8ClampedArray) => { clearTimeout(timer); worker.terminate(); value ? resolve(value) : reject(Error('无法生成荧光笔预览')); };
      const timer = setTimeout(() => finish(), 30000);
      worker.onmessage = (event: MessageEvent<{ data?: Uint8ClampedArray }>) => finish(event.data.data?.length === width * height * 4 ? event.data.data : undefined);
      worker.onerror = () => finish();
      try { worker.postMessage({ data: input.data, width, height }, [input.data.buffer]); } catch { finish(); }
    });
    const output = context.createImageData(width, height); output.data.set(data); context.putImageData(output, 0, 0);
    const dataUrl = canvas.toDataURL('image/png');
    if (dataUrl.length > Math.ceil(16 * 1024 * 1024 * 4 / 3) + 64) throw Error('荧光笔预览过大');
    return { backgroundId, x, y, width, height, dataUrl };
  } finally { canvas.width = canvas.height = 1; }
}
