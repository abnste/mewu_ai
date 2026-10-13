// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, untrack } from 'solid-js';
import type { ConnectionProfile } from './contracts';
import type { PluginSnapshot } from './plugin-contracts';
import { getOptionalCapabilities, getPlugins, subscribePlugins } from './plugin-bridge';
import { currentOptionalCapabilities, optionalCapabilityKey, type OptionalCapabilities } from './optional-capabilities';

/** Only configuration revisions trigger credential checks, never drawing updates. */
export function createOptionalFeatureState(connections: () => ConnectionProfile[], suppliedPlugins?: () => PluginSnapshot) {
  const [installed, setInstalled] = createSignal<PluginSnapshot>({ revision: -1, plugins: [] });
  const plugins = suppliedPlugins ?? installed;
  const [result, setResult] = createSignal<{ key: string; value: OptionalCapabilities }>();
  const key = createMemo(() => optionalCapabilityKey(plugins(), connections()));
  let request = 0, disposed = false, stop: (() => void) | undefined;
  createEffect(on(key, expected => {
    const serial = ++request;
    if (untrack(plugins).revision < 0) return;
    void getOptionalCapabilities().then(value => {
      if (!disposed && request === serial && expected === key()) setResult({ key: expected, value });
    }).catch(() => { if (!disposed && request === serial) setResult(undefined); });
  }));
  if (!suppliedPlugins) onMount(async () => {
    const accept = (value: PluginSnapshot) => { if (!disposed && value.revision >= installed().revision) setInstalled(value); };
    try {
      stop = await subscribePlugins(accept);
      if (disposed) { stop(); return; }
      accept(await getPlugins());
    } catch { if (!disposed) setResult(undefined); }
  });
  onCleanup(() => { disposed = true; request++; stop?.(); });
  return createMemo(() => {
    const value = result();
    return value?.key === key() && currentOptionalCapabilities(value.value, plugins(), connections()) ? value.value : undefined;
  });
}
