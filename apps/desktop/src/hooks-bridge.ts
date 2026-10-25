// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { HookRule, HookSnapshot } from './hooks-contracts';
export const hooksNative = isTauri();
export const getHooks = () => invoke<HookSnapshot>('get_hooks');
export const saveHook = (expectedRevision: number, rule: HookRule) => invoke<HookSnapshot>('save_hook', { expectedRevision, rule });
export const removeHook = (expectedRevision: number, id: string, ruleRevision: number) => invoke<HookSnapshot>('remove_hook', { expectedRevision, id, ruleRevision });
export const checkHook = (rule: HookRule) => invoke<void>('check_hook', { rule });
export const runHook = (id: string, ruleRevision: number, text: string) => invoke<string>('run_hook', { id, ruleRevision, text });
export const cancelHook = (runId: string) => invoke<void>('cancel_hook', { runId });
export const editHookShortcut = (leaseId: string, active: boolean) => invoke<void>('edit_hook_shortcut', { leaseId, active });
export async function subscribeHooks(changed: () => void, recorded: (accelerator: string) => void): Promise<() => void> {
  const stops: (() => void)[] = [];
  try {
    stops.push(await listen('hooks-changed', changed, { target: 'settings' }));
    stops.push(await listen<{ accelerator: string }>('hooks-shortcut-recorded', e => recorded(e.payload.accelerator), { target: 'settings' }));
  } catch (e) { stops.forEach(stop => stop()); throw e; }
  return () => stops.forEach(stop => stop());
}
