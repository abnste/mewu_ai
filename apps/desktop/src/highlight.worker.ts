// SPDX-License-Identifier: MPL-2.0
import { highlightForeground, straightForeground } from './highlight-foreground';
self.onmessage = (event: MessageEvent<{ data: Uint8ClampedArray; width: number; height: number }>) => {
  try {
    const { data, width, height } = event.data;
    const output = straightForeground(highlightForeground(data, width, height));
    self.postMessage({ data: output }, { transfer: [output.buffer] });
  } catch { self.postMessage({ error: '无法生成荧光笔预览' }); }
};
