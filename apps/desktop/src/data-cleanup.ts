// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, on, onCleanup, type Accessor } from 'solid-js';
import type { DataBucket, DataCategory, DataCleanupReceipt, DataUsage } from './settings-contracts';
import type { SettingsClient } from './settings-client';

export function formatStorageBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const index = Math.min(4, Math.floor(Math.log(bytes) / Math.log(1024)));
  return `${Number((bytes / 1024 ** index).toFixed(index ? 1 : 0))} ${['B', 'KiB', 'MiB', 'GiB', 'TiB'][index]}`;
}
export function createDataCleanup(api: Pick<SettingsClient, 'dataUsage' | 'cleanData'>, active: Accessor<boolean>) {
  const [usage, setUsage] = createSignal<DataUsage>(), [review, setReview] = createSignal<DataBucket>();
  const [pending, setPending] = createSignal(false), [error, setError] = createSignal(''), [receipt, setReceipt] = createSignal<DataCleanupReceipt>();
  let disposed = false;
  onCleanup(() => { disposed = true; });
  async function load() {
    if (disposed || pending()) return;
    setPending(true); setError(''); setReview(undefined);
    try { const result = await api.dataUsage(); if (!disposed) setUsage(result); }
    catch (cause) { if (!disposed) { setUsage(undefined); setError(cause instanceof Error ? cause.message : String(cause)); } }
    finally { if (!disposed) setPending(false); }
  }
  function choose(category: DataCategory) {
    const bucket = usage()?.[category];
    if (disposed || !active() || pending() || !usage()?.canClean || !bucket?.cleanableCount) return;
    setReceipt(undefined); setReview({ ...bucket }); setError('');
  }
  function cancel() { if (!pending()) setReview(undefined); }
  async function confirm() {
    const selected = review();
    if (disposed || !active() || pending() || !selected || !usage()?.canClean) return;
    setPending(true); setError('');
    try {
      const result = await api.cleanData(selected.category, selected.token);
      if (!disposed) { setReceipt(result); setUsage(result.usage); setReview(undefined); }
    } catch (cause) {
      if (!disposed) {
        setReview(undefined); setError(cause instanceof Error ? cause.message : String(cause));
        // Refresh after partial/stale/failed operations, preserving the actual
        // error. Never retry a destructive operation behind the user's back.
        try { const result = await api.dataUsage(); if (!disposed) setUsage(result); }
        catch { if (!disposed) setUsage(undefined); }
      }
    } finally { if (!disposed) setPending(false); }
  }
  createEffect(on(active, visible => { if (!visible) setReview(undefined); }));
  return { usage, review, pending, error, receipt, load, choose, cancel, confirm };
}
