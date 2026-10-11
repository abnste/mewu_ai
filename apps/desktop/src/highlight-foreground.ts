// SPDX-License-Identifier: MPL-2.0
// Same source-pixel separation as the native compositor. Browser previews use
// a disposable worker; image-sized morphology buffers are confined to tiles.
const radius = 6, tileSize = 512, halo = radius * 2 + 1;
export function highlightForeground(source: Uint8ClampedArray, width: number, height: number): Uint8ClampedArray {
  if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1 || width > 16384 || height > 16384 || width * height > 32 * 1024 * 1024 || source.length !== width * height * 4) throw Error('荧光笔图片尺寸无效');
  const output = new Uint8ClampedArray(source.length);
  for (let top = 0; top < height; top += tileSize) for (let left = 0; left < width; left += tileSize) {
    const w = Math.min(tileSize, width - left), h = Math.min(tileSize, height - top);
    const sx = Math.max(0, left - halo), sy = Math.max(0, top - halo), right = Math.min(width, left + w + halo), bottom = Math.min(height, top + h + halo);
    const tw = right - sx, th = bottom - sy, input = new Uint8ClampedArray(tw * th * 4);
    for (let y = 0; y < th; y++) input.set(source.subarray(((sy + y) * width + sx) * 4, ((sy + y) * width + right) * 4), y * tw * 4);
    const data = separate(input, tw, th);
    for (let y = 0; y < h; y++) output.set(data.subarray(((top + y - sy) * tw + left - sx) * 4, ((top + y - sy) * tw + left - sx + w) * 4), ((top + y) * width + left) * 4);
  }
  return output;
}
function separate(original: Uint8ClampedArray, width: number, height: number) {
  const count = width * height, channel = new Uint8Array(count), first = new Uint8Array(count), second = new Uint8Array(count), scratch = new Uint8Array(count);
  const protection = new Uint8Array(count), work = new Uint8Array(count * 4), foreground = new Uint8ClampedArray(count * 4);
  for (let component = 0; component < 3; component++) {
    for (let p = 0; p < count; p++) channel[p] = original[p * 4 + component];
    mean(channel, scratch, work, width, height, radius * 2);
    morphology(channel, first, scratch, width, height, radius, false);
    for (let p = 0; p < count; p++) work[p * 4 + 1] = first[p];
    morphology(first, second, scratch, width, height, radius, true);
    for (let p = 0; p < count; p++) work[p * 4 + 3] = second[p];
    morphology(channel, first, scratch, width, height, radius, true);
    for (let p = 0; p < count; p++) work[p * 4 + 2] = first[p];
    morphology(first, second, scratch, width, height, radius, false);
    for (let p = 0; p < count; p++) {
      const b = p * 4, background = Math.abs(work[b] - work[b + 1]) < Math.abs(work[b] - work[b + 2]) ? work[b + 3] : second[p];
      protection[p] = Math.max(protection[p], Math.abs(channel[p] - background)); foreground[b + component] = background;
    }
  }
  morphology(protection, first, scratch, width, height, 1, true);
  for (let p = 0; p < count; p++) {
    const b = p * 4, contrast = first[p], residual = protection[p];
    let a = contrast < 2 ? 0 : contrast - residual <= 1 ? 255 : Math.floor((residual * 255 + contrast / 2) / contrast);
    for (let c = 0; c < 3; c++) {
      const value = original[b + c], background = foreground[b + c];
      if (value < background) a = Math.max(a, Math.floor(((background - value) * 255 + background - 1) / background));
      else if (value > background) a = Math.max(a, Math.floor(((value - background) * 255 + 254 - background) / (255 - background)));
    }
    for (let c = 0; c < 3; c++) foreground[b + c] = Math.max(0, Math.min(a, Math.trunc((original[b + c] * 255 - (255 - a) * foreground[b + c] + 127) / 255)));
    foreground[b + 3] = original[b + 3] === 255 ? a : 255;
  }
  return foreground;
}
function mean(source: Uint8Array, scratch: Uint8Array, output: Uint8Array, width: number, height: number, r: number) {
  for (let y = 0; y < height; y++) {
    const row = y * width; let sum = 0;
    for (let x = 0; x <= Math.min(r, width - 1); x++) sum += source[row + x];
    for (let x = 0; x < width; x++) {
      const count = Math.min(width - 1, x + r) - Math.max(0, x - r) + 1;
      scratch[row + x] = Math.floor((sum + Math.floor(count / 2)) / count);
      if (x >= r) sum -= source[row + x - r]; if (x + r + 1 < width) sum += source[row + x + r + 1];
    }
  }
  for (let x = 0; x < width; x++) {
    let sum = 0; for (let y = 0; y <= Math.min(r, height - 1); y++) sum += scratch[y * width + x];
    for (let y = 0; y < height; y++) {
      const count = Math.min(height - 1, y + r) - Math.max(0, y - r) + 1;
      output[(y * width + x) * 4] = Math.floor((sum + Math.floor(count / 2)) / count);
      if (y >= r) sum -= scratch[(y - r) * width + x]; if (y + r + 1 < height) sum += scratch[(y + r + 1) * width + x];
    }
  }
}
function morphology(source: Uint8Array, target: Uint8Array, scratch: Uint8Array, width: number, height: number, r: number, maximum: boolean) {
  const deque = new Uint32Array(Math.max(width, height));
  for (let y = 0; y < height; y++) line(source, scratch, y * width, 1, width, r, maximum, deque);
  for (let x = 0; x < width; x++) line(scratch, target, x, width, height, r, maximum, deque);
}
function line(source: Uint8Array, target: Uint8Array, start: number, step: number, length: number, r: number, maximum: boolean, deque: Uint32Array) {
  let head = 0, tail = 0, next = 0;
  for (let p = 0; p < length; p++) {
    const end = Math.min(length - 1, p + r);
    while (next <= end) {
      const value = source[start + next * step];
      while (tail > head && (maximum ? source[start + deque[tail - 1] * step] <= value : source[start + deque[tail - 1] * step] >= value)) tail--;
      deque[tail++] = next++;
    }
    while (head < tail && deque[head] < Math.max(0, p - r)) head++;
    target[start + p * step] = source[start + deque[head] * step];
  }
}
export function straightForeground(data: Uint8ClampedArray) {
  for (let b = 0; b < data.length; b += 4) {
    const a = data[b + 3]; for (let c = 0; c < 3; c++) data[b + c] = a === 0 ? 0 : Math.min(255, Math.floor((data[b + c] * 255 + Math.floor(a / 2)) / a));
  }
  return data;
}
