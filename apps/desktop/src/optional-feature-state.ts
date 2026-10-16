// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, untrack } from 'solid-js';
import type { ConnectionProfile } from './contracts';
import type { PluginSnapshot } from './plugin-contracts';
import { getOptionalCapabilities, getPlugins, subscribePlugins } from './plugin-bridge';
import { subscribeExtensionBindings, subscribeExtensions } from './extension-bridge';
import { currentOptionalCapabilities, optionalCapabilityKey, type OptionalAgentConfiguration, type OptionalCapabilities } from './optional-capabilities';

/** Only configuration revisions trigger host metadata queries, never drawing updates or secret reads. */
export function createOptionalFeatureState(connections: () => ConnectionProfile[], suppliedPlugins?: () => PluginSnapshot, configuration?: () => OptionalAgentConfiguration) {
  const [installed, setInstalled] = createSignal<PluginSnapshot>({ revision: -1, plugins: [] });
  const plugins = suppliedPlugins ?? installed;
  const [result, setResult] = createSignal<{ key: string; value: OptionalCapabilities }>();
  const [extensionEpoch, setExtensionEpoch] = createSignal(0), [listening, setListening] = createSignal(false);
  const key = createMemo(() => JSON.stringify([optionalCapabilityKey(plugins(), connections(), configuration?.()), extensionEpoch(), listening()]));
  let request = 0, disposed = false, stop: (() => void) | undefined, stopExtensions: (() => void) | undefined, stopBindings: (() => void) | undefined;
  createEffect(on(key, expected => {
    const serial = ++request;
    if (!listening() || untrack(plugins).revision < 0) return;
    void getOptionalCapabilities().then(value => {
      if (!disposed && request === serial && expected === key()) setResult({ key: expected, value });
    }).catch(() => { if (!disposed && request === serial) setResult(undefined); });
  }));
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
      stop = await subscribePlugins(accept);
      if (disposed) { stop(); return; }
      accept(await getPlugins());
    } catch { if (!disposed) setResult(undefined); }
  });
  onCleanup(() => { disposed = true; request++; stop?.(); stopExtensions?.(); stopBindings?.(); });
  return createMemo(() => {
    const value = result();
    return value?.key === key() && currentOptionalCapabilities(value.value, plugins(), connections(), configuration?.()) ? value.value : undefined;
  });
}
