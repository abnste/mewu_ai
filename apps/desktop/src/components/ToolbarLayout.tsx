// SPDX-License-Identifier: MPL-2.0
import { children, createEffect, createMemo, createSignal, For, onCleanup, onMount, type JSX } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import * as api from '../extension-bridge';
import { mergeExtensionRecords } from '../extension-state';
import type { ExtensionRecord } from '../extension-contracts';
import { reorderToolbar, toolbarLayoutServices } from '../toolbar-layout';

/** Move existing nodes through Solid's keyed list, never rebuild their handlers
 * or use CSS order (which would leave keyboard navigation in the wrong order). */
export default function ToolbarLayout(props: { surface: string; children: JSX.Element }) {
  const resolved = children(() => props.children);
  const nodes = createMemo(() => resolved.toArray());
  const [records, setRecords] = createSignal<ExtensionRecord[]>([]), [refresh, setRefresh] = createSignal(0);
  const [plans, setPlans] = createSignal<unknown[]>([]);
  let disposed = false, generation = 0, events = 0;
  const stops: Array<() => void> = [];
  const id = (node: JSX.Element) => node instanceof HTMLElement ? node.dataset.layoutId : undefined;
  const items = createMemo(() => nodes().flatMap(node => { const key = id(node); return key ? [{ id: key, title: (node as HTMLElement).getAttribute('data-caption') || (node as HTMLElement).getAttribute('aria-label') || key }] : []; }), undefined, { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) });
  const ordered = createMemo(() => plans().reduce<JSX.Element[]>((value, result) => reorderToolbar(value, id, result), nodes()));
  let owner = '';
  onMount(async () => {
    if (!api.extensionNative) return;
    try {
      const stop = await api.subscribeExtensions(values => { events++; if (!disposed) setRecords(old => mergeExtensionRecords(old, values)); });
      if (disposed) { stop(); return; } stops.push(stop);
      const unlisten = await listen('extension-configuration-changed', () => setRefresh(v => v + 1));
      if (disposed) { unlisten(); return; } stops.push(unlisten);
      const before = events, values = await api.getExtensions();
      if (!disposed && before === events) setRecords(values);
    } catch { /* Existing screenshot controls remain fully usable offline. */ }
  });
  createEffect(() => {
    const source = items(), services = toolbarLayoutServices(records(), props.surface), surface = props.surface;
    refresh(); const current = ++generation;
    const identity = JSON.stringify([surface, services.map(c => [c.record.manifest.id, c.record.revision, c.service.id])]);
    if (owner !== identity) { owner = identity; setPlans([]); }
    if (!services.length) return;
    void (async () => {
      let value = source; const next: unknown[] = [];
      try {
        for (const choice of services) {
          const result = await api.invokeExtension({ pluginId: choice.record.manifest.id, revision: choice.record.revision, serviceId: choice.service.id, input: { type: 'layout', surface, items: value } });
          if (disposed || current !== generation) return;
          value = reorderToolbar(value, item => item.id, result); next.push(result);
        }
        if (!disposed && current === generation) setPlans(next);
      } catch { if (!disposed && current === generation) setPlans([]); }
    })();
  });
  onCleanup(() => { disposed = true; generation++; stops.forEach(stop => stop()); });
  return <For each={ordered()}>{node => node}</For>;
}
