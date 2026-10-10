// SPDX-License-Identifier: MPL-2.0
export function cancelPinOnEscape(event: Pick<KeyboardEvent, 'key' | 'preventDefault' | 'stopImmediatePropagation'>, pending: boolean, cancel: () => void): boolean {
  if (event.key !== 'Escape' || !pending) return false;
  cancel(); event.preventDefault(); event.stopImmediatePropagation(); return true;
}
export async function preparePin<T>(actions: { prepare: () => Promise<void>; target: () => T | undefined; run: (target: T) => Promise<unknown> }): Promise<boolean> {
  await actions.prepare(); const target = actions.target(); if (target === undefined) return false;
  await actions.run(target); return true;
}
export class PinDragGesture {
  private press?: { x: number; y: number; pointerId: number };
  begin(x: number, y: number, pointerId: number) { this.press = Number.isFinite(x) && Number.isFinite(y) ? { x, y, pointerId } : undefined; }
  move(x: number, y: number, pointerId: number, buttons: number, thresholdX: number, thresholdY: number): boolean {
    const start = this.press;
    if (!start || start.pointerId !== pointerId) return false;
    if (!(buttons & 1) || ![x, y, thresholdX, thresholdY].every(Number.isFinite) || thresholdX < 0 || thresholdY < 0) { this.end(); return false; }
    if (Math.abs(x - start.x) < thresholdX && Math.abs(y - start.y) < thresholdY) return false;
    this.end(); return true;
  }
  end() { this.press = undefined; }
}
export function pinImageLayout(width: number, height: number, quarterTurns: number, boxWidth: number, boxHeight: number) {
  if (![width, height, quarterTurns, boxWidth, boxHeight].every(Number.isFinite) || width <= 0 || height <= 0 || boxWidth <= 0 || boxHeight <= 0 || !Number.isInteger(quarterTurns)) return;
  const turns = ((quarterTurns % 4) + 4) % 4;
  const scale = Math.min(boxWidth / (turns % 2 ? height : width), boxHeight / (turns % 2 ? width : height));
  return { width: width * scale, height: height * scale, angle: turns * 90 };
}
export class PinRevisionGate {
  private id?: string; private revision = -1; private disposed = false;
  accept(value: { id: string; revision: number }): boolean {
    if (this.disposed || !value.id || !Number.isSafeInteger(value.revision) || value.revision < this.revision || (this.id !== undefined && this.id !== value.id)) return false;
    this.id = value.id; this.revision = value.revision; return true;
  }
  dispose() { this.disposed = true; }
}
