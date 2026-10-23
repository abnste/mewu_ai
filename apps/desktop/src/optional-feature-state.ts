// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, untrack } from 'solid-js';
import type { ConnectionProfile } from './contracts';
import type { PluginSnapshot } from './plugin-contracts';
import { getOptionalCapabilities, getPlugins, subscribePlugins } from './plugin-bridge';
import { subscribeExtensionBindings, subscribeExtensions } from './extension-bridge';
import { currentOptionalCapabilities, optionalCapabilityKey, type OptionalAgentConfiguration, type OptionalCapabilities } from './optional-capabilities';

/** Only configuration revisions trigger host metadata queries, never drawing updates or secret reads. */
export function createOptionalFeatureState(connections: () => ConnectionProfile[], suppliedPlugins?: () => PluginSnapshot, configuration?: () => OptionalAgentConfiguration, surface: 'space' | 'settings' = 'space') {
  const [installed, setInstalled] = createSignal<PluginSnapshot>({ revision: -1, plugins: [] });
  const plugins = suppliedPlugins ?? installed;
  const [result, setResult] = createSignal<{ key: string; value: OptionalCapabilities }>();
  const [extensionEpoch, setExtensionEpoch] = createSignal(0), [listening, setListening] = createSignal(false);
  const key = createMemo(() => JSON.stringify([optionalCapabilityKey(plugins(), connections(), configuration?.()), extensionEpoch(), listening()]));
  let request = 0, disposed = false, stop: (() => void) | undefined, stopExtensions: (() => void) | undefined, stopBindings: (() => void) | undefined;
  const refresh = async (expected = untrack(key)) => {
    const serial = ++request;
    if (!listening() || untrack(plugins).revision < 0) return;
    try {
      const value = await getOptionalCapabilities();
      if (!disposed && request === serial && expected === key()) setResult({ key: expected, value });
    } catch { if (!disposed && request === serial) setResult(undefined); }
  };
  createEffect(on(key, expected => { void refresh(expected); }));
  onMount(async () => {
    const accept = (value: PluginSnapshot) => { if (!disposed && value.revision >= installed().revision) setInstalled(value); };
    try {
      const changed = () => { if (!disposed) setExtensionEpoch(value => value + 1); };
      stopExtensions = await subscribeExtensions(changed);
      if (disposed) { stopExtensions(); return; }
      stopBindings = await subscribeExtensionBindings(changed);
      if (disposed) { stopBindings(); return; }
      setListening(true);
      if (suppliedPlugins) return;
      stop = await subscribePlugins(accept, surface);
      if (disposed) { stop(); return; }
      accept(await getPlugins());
    } catch { if (!disposed) setResult(undefined); }
  });
  onCleanup(() => { disposed = true; request++; stop?.(); stopExtensions?.(); stopBindings?.(); });
  const capabilities = createMemo(() => {
    const value = result();
    return value?.key === key() && currentOptionalCapabilities(value.value, plugins(), connections(), configuration?.()) ? value.value : undefined;
  });
  // Presentation may retain an editor across metadata refreshes, but a known
  // plugin revocation must take effect immediately, before that request ends.
  // An explicit action may recover a failed cold-start metadata read. This is
  // one authoritative query, never a replay of an operation or a polling loop.
  return Object.assign(capabilities, { installed: plugins, refresh: () => refresh() });
}
