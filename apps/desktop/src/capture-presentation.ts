// SPDX-License-Identifier: MPL-2.0
export interface CaptureBackgroundReady {
  sceneId: string;
  backgroundId: string;
  width: number;
  height: number;
  success: boolean;
}

/** Decode the actual displayed element, then flush its layout. A hidden WebView
 * can suspend requestAnimationFrame, so readiness must not wait for a frame. */
export async function prepareCaptureBackground(
  image: HTMLImageElement,
  identity: Omit<CaptureBackgroundReady, 'success'>,
  current: () => boolean,
  report: (ready: CaptureBackgroundReady) => Promise<unknown>,
): Promise<void> {
  const owns = () => current() && image.isConnected && image.dataset.asset === identity.backgroundId;
  if (!owns()) return;
  try { await image.decode(); }
  catch { if (owns()) await report({ ...identity, success: false }); return; }
  if (!owns()) return;
  // This read synchronously computes the layout after natural dimensions and
  // the native viewport resize have been applied; it does not resample pixels.
  const bounds = image.getBoundingClientRect();
  const success = image.complete && image.naturalWidth === identity.width
    && image.naturalHeight === identity.height && bounds.width > 0 && bounds.height > 0;
  if (owns()) await report({ ...identity, success });
}
