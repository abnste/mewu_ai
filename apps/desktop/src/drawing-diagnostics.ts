// SPDX-License-Identifier: MPL-2.0
// Advisory input observations only. No coordinates, identities, text or URLs.
export const DrawingInputPhase = {
  pointerDown: 1, mouseDown: 2, pointerUp: 3, mouseUp: 4, focus: 5, blur: 6,
  begin: 10, rejected: 11, admitted: 12, canceled: 13, finished: 14,
  enqueued: 15, history: 16, style: 17,
  windowSnapMap: 20, windowSnapDown: 21,
  regionBegin: 22, regionFinish: 23, regionEdit: 24,
} as const;
export type DrawingInputPhaseValue = typeof DrawingInputPhase[keyof typeof DrawingInputPhase];
export interface DrawingInputState { flags: number }
interface Row {
  sequence: number; atMicros: number; phase: DrawingInputPhaseValue;
  target: number; active: number; button: number; buttons: number;
  pointer: number; focused: boolean; flags: number; code: number; count: number;
}
const MAX_ROWS = 256, MAX_BATCH = 16, MAX_MICROS = 600_000_000;
let enabled = false, generation = 0, sequence = 0, origin: number | undefined;
let rows: Row[] = [], timer: ReturnType<typeof setTimeout> | undefined;
let sender: ((payload: string) => Promise<unknown>) | undefined;
let flight: Promise<unknown> | undefined, detach: (() => void) | undefined;

/** A boolean-only gate; callers do not inspect DOM or clocks when disabled. */
export function drawingDiagnosticsEnabled(): boolean { return enabled && sequence < MAX_ROWS; }

function target(value: EventTarget | null | undefined): number {
  if (!(value instanceof Element)) return 0;
  if (value.closest('.drawing-recent-color')) return 3;
  if (value.closest('.drawing-edit-surface')) return 1;
  if (value.closest('.drawing-toolbar')) return 2;
  if (value.closest('.drawing-editor')) return 5;
  if (value.closest('button')) return 4;
  if (value.closest('.capture-handle,.capture-edge')) return 10;
  if (value.closest('.capture-region')) return 9;
  if (value.closest('.capture-surface')) return 8;
  if (value === document.body || value === document.documentElement) return 6;
  return 7;
}
function integer(value: unknown, minimum: number, maximum: number): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= minimum && value <= maximum;
}
function stop(): void {
  enabled = false; generation++; rows = []; sender = undefined;
  try { if (timer !== undefined) clearTimeout(timer); } catch { /* Advisory only. */ }
  timer = undefined;
  try { detach?.(); } catch { /* Diagnostics cannot change input handling. */ }
  detach = undefined;
}
function pump(): void {
  timer = undefined;
  if (!enabled || flight || !rows.length || !sender) return;
  const batch = rows.splice(0, MAX_BATCH), token = generation, send = sender;
  const pending = Promise.resolve().then(() => {
    if (enabled && token === generation) return send(JSON.stringify({ version: 1, rows: batch }));
  });
  flight = pending;
  void pending.catch(() => { if (token === generation) stop(); }).finally(() => {
    if (flight === pending) flight = undefined;
    if (enabled && rows.length) schedule();
  });
}
function schedule(): void {
  try { if (timer === undefined && !flight) timer = setTimeout(pump, 120); }
  catch { stop(); }
}
/** Lazy state is not evaluated unless the native runtime explicitly opted in. */
export function recordDrawingInput(phase: DrawingInputPhaseValue, event?: Event,
  state?: () => DrawingInputState, code = 0, count = 0): void {
  if (!enabled || sequence >= MAX_ROWS) return;
  try {
    const next = state?.().flags ?? 0, input = event as PointerEvent | undefined;
    const button = input?.button ?? -1, buttons = input?.buttons ?? 0;
    const atMicros = Math.round((performance.now() - origin!) * 1000);
    if (!integer(next, 0, 4095) || !integer(code, 0, 31) || !integer(count, 0, 4096)
      || !integer(button, -1, 5) || !integer(buttons, 0, 31) || !integer(atMicros, 0, MAX_MICROS)) return;
    rows.push({ sequence: ++sequence, atMicros, phase, target: target(event?.target),
      active: target(document.activeElement), button, buttons,
      pointer: input?.pointerType === 'mouse' ? 1 : input?.pointerType === 'pen' ? 2 : input?.pointerType === 'touch' ? 3 : 0,
      focused: document.hasFocus(), flags: next, code, count });
    if (sequence === MAX_ROWS) { try { detach?.(); } catch {} detach = undefined; }
    schedule();
  } catch { /* Observers cannot replace a business result or exception. */ }
}
/** Called only for the first-party space after native runtime_info succeeds.
 * Disabled mode installs nothing and reads no clock or DOM. */
export function startDrawingDiagnostics(value: boolean, send: (payload: string) => Promise<unknown>): () => void {
  stop();
  if (value !== true || sequence >= MAX_ROWS) return () => {};
  try {
    if (origin === undefined) origin = performance.now();
    sender = send; enabled = true;
    const listeners: [EventTarget, string, EventListener][] = [];
    detach = () => { for (const [owner, name, listener] of listeners) owner.removeEventListener(name, listener, true); };
    const observe = (owner: EventTarget, name: string, phase: DrawingInputPhaseValue) => {
      const listener = (event: Event) => recordDrawingInput(phase, event);
      listeners.push([owner, name, listener]); owner.addEventListener(name, listener, true);
    };
    observe(document, 'pointerdown', DrawingInputPhase.pointerDown);
    observe(document, 'mousedown', DrawingInputPhase.mouseDown);
    observe(document, 'pointerup', DrawingInputPhase.pointerUp);
    observe(document, 'mouseup', DrawingInputPhase.mouseUp);
    observe(window, 'focus', DrawingInputPhase.focus); observe(window, 'blur', DrawingInputPhase.blur);
    const token = generation;
    return () => { if (token === generation) stop(); };
  } catch { stop(); return () => {}; }
}
