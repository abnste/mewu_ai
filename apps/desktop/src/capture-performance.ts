// SPDX-License-Identifier: MPL-2.0
import type { CaptureFrontendTiming, CapturePreparationDiagnostics } from './capture-presentation';

interface Identity { sceneId: string; backgroundId: string; width: number; height: number }
interface Source { activeSceneId: string; scenes: { id: string; closed?: boolean; frozen: boolean; background?: { id: string; width?: number; height?: number } }[] }
interface Pending { identity: Identity; started: number; documentHidden: boolean; marks: number[] }
let enabled = false;
let pending: Pending | undefined;
let last: Identity | undefined;
const same = (a: Identity, b: Identity) => a.sceneId === b.sceneId && a.backgroundId === b.backgroundId && a.width === b.width && a.height === b.height;
const MAX_MICROS = 8_000_000;

/** Only the native runtime's existing opt-in switch enables clocks. At most one
 * pending identity is retained; no Snapshot, Asset, image, URL or history map. */
export function configureCapturePerformance(value: boolean): void {
  enabled = value === true;
  pending = undefined;
  last = undefined;
}
export function captureSnapshotReceived(snapshot: Source): object | undefined {
  if (!enabled) return;
  try {
    const scene = snapshot.scenes.find(value => value.id === snapshot.activeSceneId), image = scene?.background;
    if (!scene || scene.closed || scene.frozen || !image?.width || !image.height) { pending = undefined; return; }
    const identity = { sceneId: scene.id, backgroundId: image.id, width: image.width, height: image.height };
    if (last && same(last, identity)) return;
    last = identity;
    pending = { identity, started: performance.now(), documentHidden: document.visibilityState === 'hidden', marks: [0] };
    return pending;
  } catch { pending = undefined; }
}
function mark(owner: Pending, index: number): boolean {
  if (!enabled || pending !== owner || owner.marks.length !== index) return false;
  try {
    const micros = Math.round((performance.now() - owner.started) * 1000);
    if (!Number.isSafeInteger(micros) || micros < owner.marks[index - 1] || micros > MAX_MICROS) { pending = undefined; return false; }
    owner.marks.push(micros);
    return true;
  } catch { pending = undefined; return false; }
}
export function captureSnapshotNormalized(token: object | undefined): void {
  if (pending && pending === token) mark(pending, 1);
}
export function capturePreparationPerformance(identity: Identity): CapturePreparationDiagnostics | undefined {
  const owner = pending;
  if (!enabled || !owner || !same(owner.identity, identity) || owner.marks.length !== 2) return;
  const positions = { attached: 2, decodeStarted: 3, imageDecoded: 4, layoutReady: 5 } as const;
  return {
    mark(stage) { mark(owner, positions[stage]); },
    finish(): CaptureFrontendTiming | undefined {
      if (!mark(owner, 6)) return;
      pending = undefined;
      const [receivedMicros, normalizedMicros, attachedMicros, decodeStartedMicros, imageDecodedMicros, layoutReadyMicros, invokeStartedMicros] = owner.marks;
      return { documentHidden: owner.documentHidden, receivedMicros, normalizedMicros, attachedMicros, decodeStartedMicros, imageDecodedMicros, layoutReadyMicros, invokeStartedMicros };
    },
  };
}
