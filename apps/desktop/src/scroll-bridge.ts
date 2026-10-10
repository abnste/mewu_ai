// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { OcrTarget } from './contracts';

export interface ScrollStatus {
  id: string; sceneId: string; phase: 'starting' | 'capturing' | 'finishing';
  rect: { x: number; y: number; width: number; height: number };
  width: number; height: number;
  status: 'initial' | 'extended' | 'retraced' | 'unchanged' | 'low_information' | 'ambiguous' | 'lost_overlap' | 'limit_reached';
  stopHotkey: string;
  preview?:string;
}
export type ScrollAction = 'finish' | 'keep' | 'cancel';
export function scrollNeedsKeep(status: ScrollStatus): boolean { return ['low_information', 'ambiguous', 'lost_overlap', 'limit_reached'].includes(status.status); }
const native = isTauri();
export async function startScrollCapture(pluginId: string, revision: number, contributionId: string, target: OcrTarget): Promise<void> {
  if (!native) throw new Error('请在桌面版使用长截图');
  return invoke('start_scroll_capture', { pluginId, revision, contributionId, target });
}
export async function controlScrollCapture(id: string, action: ScrollAction): Promise<void> {
  if (!native) throw new Error('请在桌面版使用长截图');
  return invoke('control_scroll_capture', { id, action });
}

// Native events are emitted in order. The initial query can arrive after any event.
export class ScrollStatusGate {
  private current: ScrollStatus | null = null;
  private ended = new Set<string>();
  private disposed = false;
  accept(next: ScrollStatus | null): boolean {
    if (this.disposed || (next && this.ended.has(next.id))) return false;
    const ranks = { starting: 0, capturing: 1, finishing: 2 };
    if (next && this.current?.id === next.id && ranks[next.phase] < ranks[this.current.phase]) return false;
    if (this.current && this.current.id !== next?.id) {
      this.ended.add(this.current.id);
      if (this.ended.size > 64) this.ended.delete(this.ended.values().next().value!);
    }
    this.current = next; return true;
  }
  dispose() { this.disposed = true; }
}
export async function subscribeScroll(onStatus: (status: ScrollStatus | null) => void): Promise<() => void> {
  if (!native) { onStatus(null); return () => undefined; }
  const gate = new ScrollStatusGate(); let serial = 0;
  const accept = (value: ScrollStatus | null) => { if (gate.accept(value)) onStatus(value); };
  const stop = await listen<ScrollStatus | null>('scroll-status', event => { serial++; accept(event.payload); });
  try {
    const stamp = serial, initial = await invoke<ScrollStatus | null>('get_scroll_status');
    if (stamp === serial) accept(initial);
    return () => { gate.dispose(); stop(); };
  } catch (error) { gate.dispose(); stop(); throw error; }
}
