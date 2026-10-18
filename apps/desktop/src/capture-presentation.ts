// SPDX-License-Identifier: MPL-2.0
export interface CaptureBackgroundReady {
  sceneId: string;
  backgroundId: string;
  width: number;
  height: number;
  success: boolean;
  diagnostics?: CaptureFrontendTiming;
}

export interface CaptureFrontendTiming {
  documentHidden: boolean;
  receivedMicros: number;
  normalizedMicros: number;
  attachedMicros: number;
  decodeStartedMicros: number;
  imageDecodedMicros: number;
  layoutReadyMicros: number;
  invokeStartedMicros: number;
}
export interface CapturePreparationDiagnostics {
  mark(stage: 'attached' | 'decodeStarted' | 'imageDecoded' | 'layoutReady'): void;
  finish(): CaptureFrontendTiming | undefined;
}

type Identity = Omit<CaptureBackgroundReady, 'success' | 'diagnostics'>;
interface Preparation { identity: Identity; source: string | null; promise: Promise<void> }
const preparations = new WeakMap<HTMLImageElement, Preparation>();

/** Decode the actual displayed element, then flush its layout. A hidden WebView
 * can suspend requestAnimationFrame, so readiness must not wait for a frame. */
export function prepareCaptureBackground(
  image: HTMLImageElement,
  identity: Identity,
  current: () => boolean,
  report: (ready: CaptureBackgroundReady) => Promise<unknown>,
  createDiagnostics?: () => CapturePreparationDiagnostics | undefined,
): Promise<void> {
  const source = image.getAttribute('src');
  const eligible = () => current() && image.isConnected && image.dataset.asset === identity.backgroundId
    && image.getAttribute('src') === source;
  if (!source || !eligible()) return Promise.resolve();
  const previous = preparations.get(image);
  if (previous?.source === source && previous.identity.sceneId === identity.sceneId
    && previous.identity.backgroundId === identity.backgroundId
    && previous.identity.width === identity.width && previous.identity.height === identity.height) return previous.promise;
  const preparation: Preparation = { identity: { ...identity }, source, promise: Promise.resolve() };
  preparations.set(image, preparation);
  let diagnostics: CapturePreparationDiagnostics | undefined;
  try { diagnostics = createDiagnostics?.(); } catch { /* Advisory only. */ }
  const mark = (stage: Parameters<CapturePreparationDiagnostics['mark']>[0]) => {
    try { diagnostics?.mark(stage); } catch { diagnostics = undefined; }
  };
  mark('attached');
  const owns = () => preparations.get(image) === preparation && eligible();
  // Claim this element before calling decode/report: both may settle or throw
  // immediately. Load/error callbacks must reuse this one preparation.
  preparation.promise = Promise.resolve().then(async () => {
    if (!owns()) return;
    mark('decodeStarted');
    try { await image.decode(); }
    catch { if (owns()) await report({ ...identity, success: false }); return; }
    if (!owns()) return;
    mark('imageDecoded');
    // This read synchronously computes the layout after natural dimensions and
    // the native viewport resize have been applied; it does not resample pixels.
    const bounds = image.getBoundingClientRect();
    const success = image.complete && image.naturalWidth === identity.width
      && image.naturalHeight === identity.height && bounds.width > 0 && bounds.height > 0;
    if (owns()) {
      mark('layoutReady');
      let timing: CaptureFrontendTiming | undefined;
      if (success) try { timing = diagnostics?.finish(); } catch { /* Advisory only. */ }
      await report({ ...identity, success, ...(timing ? { diagnostics: timing } : {}) });
    }
  });
  return preparation.promise;
}
