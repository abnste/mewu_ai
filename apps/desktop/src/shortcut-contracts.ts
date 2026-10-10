// SPDX-License-Identifier: MPL-2.0
export type ShortcutLetter = 'A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'G' | 'H' | 'I' | 'J' | 'K' | 'L' | 'M' | 'N' | 'O' | 'P' | 'Q' | 'R' | 'S' | 'T' | 'U' | 'V' | 'W' | 'X' | 'Y' | 'Z';
export interface ShortcutChord { code: `Key${ShortcutLetter}`; ctrl: boolean; shift: boolean; alt: boolean }
export interface CaptureShortcutState {
  revision: number;
  sequence: number;
  configured: ShortcutChord | null;
  active: ShortcutChord | null;
  status: 'active' | 'disabled' | 'fallback' | 'unavailable' | 'cleanup_pending';
  editable: boolean;
  message?: string | null;
}
export interface ShortcutEditLease { leaseId: string; expiresAt: number }
export interface ShortcutRecorded { leaseId: string; shortcut: ShortcutChord }
