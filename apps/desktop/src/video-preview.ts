// SPDX-License-Identifier: MPL-2.0
export type PreviewResolution = 50 | 75 | 100;
export function previewSize(width: number, height: number, percent: PreviewResolution) {
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1 || width > 8192 || height > 8192) return undefined;
  const scale = [50, 75, 100].includes(percent) ? percent / 100 : 1;
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}
/** Scale only the presented surface; source bytes, clock, trim and export remain unchanged. */
export function reducedVideoPreview(video: HTMLVideoElement, canvas: HTMLCanvasElement, percent: PreviewResolution, visible: (value: boolean) => void) {
  const context = percent === 100 ? null : canvas.getContext('2d', { alpha: false });
  let disposed = false, pending: number | undefined;
  function draw() {
    if (disposed || !context || video.readyState < 2) return;
    const size = previewSize(video.videoWidth, video.videoHeight, percent);
    if (!size) return;
    if (canvas.width !== size.width) canvas.width = size.width;
    if (canvas.height !== size.height) canvas.height = size.height;
    try { context.drawImage(video, 0, 0, size.width, size.height); visible(true); }
    catch { visible(false); }
  }
  function frame() { pending = undefined; if (disposed) return; draw(); schedule(); }
  function schedule() {
    if (disposed || pending !== undefined || video.paused || video.ended || !context) return;
    if (typeof video.requestVideoFrameCallback === 'function') pending = video.requestVideoFrameCallback(frame);
  }
  const update = () => { draw(); schedule(); };
  const reset = () => visible(false);
  const events = ['loadeddata', 'seeked', 'play', 'timeupdate', 'resize'] as const;
  if (context) { events.forEach(name => video.addEventListener(name, update)); video.addEventListener('emptied', reset); update(); }
  return () => {
    disposed = true;
    events.forEach(name => video.removeEventListener(name, update)); video.removeEventListener('emptied', reset);
    if (pending !== undefined) video.cancelVideoFrameCallback?.(pending);
    visible(false); canvas.width = canvas.height = 1;
  };
}
