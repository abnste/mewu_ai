// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, onCleanup, Show } from 'solid-js';
import * as extensions from '../extension-bridge';
import type { ExtensionServiceChoice } from '../extension-state';
import { surfaceBridge } from '../extension-surface-bridge';

/** Settings is a generic plugin surface. The iframe can invoke only its owner. */
export default function ExtensionSettings(props: { choice: ExtensionServiceChoice; onEscape: () => void }) {
  const [document, setDocument] = createSignal<{ surfaceId: string; url: string }>();
  const [error, setError] = createSignal('');
  let iframe: HTMLIFrameElement | undefined;
  let bridge: ReturnType<typeof surfaceBridge> | undefined;
  let bridgeSurfaceId: string | undefined;
  let generation = 0;
  const invocation = () => ({ pluginId: props.choice.record.manifest.id, revision: props.choice.record.revision, serviceId: props.choice.service.id });
  createEffect(() => {
    const owner = invocation(), ticket = ++generation;
    setDocument(undefined); setError('');
    let mounted: { surfaceId: string; url: string } | undefined;
    const release = () => {
      if (mounted && bridgeSurfaceId === mounted.surfaceId) { bridge?.close(); bridge = undefined; bridgeSurfaceId = undefined; }
      if (mounted) void extensions.invokeExtension({ ...owner, input: null, surface: { action: 'close', id: mounted.surfaceId } }).catch(() => undefined);
    };
    void extensions.invokeExtension({ ...owner, input: { type: 'mount' }, surface: { action: 'mount' } }).then(value => {
      mounted = value as typeof mounted;
      if (ticket !== generation) { release(); return; }
      if (!mounted || typeof mounted.surfaceId !== 'string' || typeof mounted.url !== 'string') throw Error('插件设置界面无效');
      setDocument(mounted);
    }).catch(cause => { if (ticket === generation) setError(cause instanceof Error ? cause.message : '无法打开插件设置'); });
    onCleanup(() => { ++generation; release(); });
  });
  const connect = () => {
    bridge?.close();
    const mounted = document(), target = iframe?.contentWindow;
    if (!mounted || !target) return;
    const owner = invocation(), channel = new MessageChannel();
    bridge = surfaceBridge(channel.port1, input => extensions.invokeExtension({ ...owner, input, surface: { action: 'call', id: mounted.surfaceId } }), () => undefined, undefined, props.onEscape, setError);
    bridgeSurfaceId = mounted.surfaceId;
    target.postMessage({ type: 'mewu-connect' }, '*', [channel.port2]);
    const style = getComputedStyle(iframe!.parentElement!);
    bridge.event('appearance', { colorScheme: style.colorScheme === 'dark' ? 'dark' : 'light', colors: {
      text: style.getPropertyValue('--text').trim(), muted: style.getPropertyValue('--secondary').trim(),
      field: style.getPropertyValue('--surface-subtle').trim(), line: style.getPropertyValue('--line').trim(),
      accent: style.getPropertyValue('--accent').trim(),
    } });
  };
  onCleanup(() => bridge?.close());
  return <section class="extension-settings-page">
    <Show when={error()}><p role="alert" class="form-error">{error()}</p></Show>
    <Show when={document()}>{value => <iframe ref={iframe} title={props.choice.service.title} src={value().url} sandbox="allow-scripts" allow="" referrerpolicy="no-referrer" onLoad={connect} />}</Show>
  </section>;
}
