// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { LoaderCircle, Square, X } from 'lucide-solid';
import PluginIcon from './PluginIcon';
import * as api from '../extension-bridge';
import type { ExtensionRecord } from '../extension-contracts';
import { extensionCommandInput, extensionResultText, extensionServiceKey, mergeExtensionRecords, spaceExtensionServices, type ExtensionServiceChoice } from '../extension-state';
import { t } from '../i18n';
import '../extension-ui.css';

interface Props { sceneId: string; disabled: boolean; onError: (message: string) => void }
interface Invocation { choice: ExtensionServiceChoice; sceneId: string; canceled: boolean; finished: boolean; stopping?: Promise<void> }

export default function ExtensionSpaceCommands(props: Props) {
  const [records, setRecords] = createSignal<ExtensionRecord[]>([]);
  const [running, setRunning] = createSignal<ExtensionServiceChoice>(), [stopping, setStopping] = createSignal(false);
  const [result, setResult] = createSignal('');
  const services = createMemo(() => spaceExtensionServices(records()));
  const buttons = createMemo(() => { const available = services(), executing = running(); return executing && !available.some(choice => extensionServiceKey(choice) === extensionServiceKey(executing)) ? [...available, executing] : available; });
  const sceneId = createMemo(() => props.sceneId), disabled = createMemo(() => props.disabled);
  let disposed = false, eventSerial = 0, active: Invocation | undefined, element: HTMLSpanElement | undefined;
  let unsubscribe: (() => void) | undefined;
  const accept = (value: ExtensionRecord[]) => { if (!disposed) setRecords(old => mergeExtensionRecords(old, value)); };
  const current = (value: Invocation) => !disposed && active === value && !value.canceled && props.sceneId === value.sceneId && !props.disabled;
  const release = (value: Invocation) => {
    if (active !== value || !value.finished || value.stopping) return;
    active = undefined;
    if (!disposed) { setRunning(undefined); setStopping(false); }
  };
  createEffect(on(() => [sceneId(), disabled()] as const, () => {
    setResult('');
    if (active && (props.disabled || active.sceneId !== props.sceneId)) void stop();
  }, { defer: true }));
  createEffect(() => {
    const available = services(), executing = running();
    if (!available.length) setResult('');
    if (executing && !available.some(choice => extensionServiceKey(choice) === extensionServiceKey(executing) && choice.record.revision === executing.record.revision)) void stop();
  });
  if (api.extensionNative) void (async () => {
    try {
      unsubscribe = await api.subscribeExtensions(value => { eventSerial++; accept(value); });
      if (disposed) { unsubscribe(); return; }
      const beforeEvents = eventSerial, installed = await api.getExtensions();
      if (beforeEvents === eventSerial) accept(installed);
    } catch (cause) { if (!disposed) props.onError(cause instanceof Error ? cause.message : String(cause)); }
  })();
  function outside(event: PointerEvent) {
    if (element && !element.contains(event.target as Node)) setResult('');
  }
  if (typeof document !== 'undefined') document.addEventListener('pointerdown', outside);
  onCleanup(() => {
    disposed = true; unsubscribe?.();
    if (typeof document !== 'undefined') document.removeEventListener('pointerdown', outside);
    if (active) void stop();
  });
  async function execute(choice: ExtensionServiceChoice) {
    if (disposed || props.disabled || active) return;
    const live = services().find(value => extensionServiceKey(value) === extensionServiceKey(choice));
    if (!live || live.unavailableReason || live.record.revision !== choice.record.revision) { props.onError(t(live?.unavailableReason || '插件状态已变化')); return; }
    const invocation: Invocation = { choice: live, sceneId: props.sceneId, canceled: false, finished: false };
    active = invocation; setResult(''); setRunning(live);
    try {
      // Space commands receive a scene, never an implicit screen-region grant.
      const value = await api.invokeExtension({ pluginId: live.record.manifest.id, revision: live.record.revision,
        serviceId: live.service.id, input: extensionCommandInput(live.service), sceneId: invocation.sceneId });
      if (current(invocation)) setResult(extensionResultText(value));
    } catch (cause) { if (current(invocation)) props.onError(cause instanceof Error ? cause.message : String(cause)); }
    finally { invocation.finished = true; release(invocation); }
  }
  async function stop() {
    const invocation = active;
    if (!invocation) return;
    if (invocation.stopping) return invocation.stopping.catch(() => {});
    invocation.canceled = true;
    if (!disposed) { setResult(''); setStopping(true); }
    // Keep the slot until both acknowledgements arrive: a late Stop must not
    // cancel a newer command from the same plugin.
    invocation.stopping = Promise.resolve().then(() => api.stopExtension(invocation.choice.record.manifest.id));
    try { await invocation.stopping; }
    catch (cause) { if (!disposed && active === invocation && props.sceneId === invocation.sceneId) props.onError(cause instanceof Error ? cause.message : String(cause)); }
    finally { invocation.stopping = undefined; if (!disposed && active === invocation) setStopping(false); release(invocation); }
  }
  function keyboard(event: KeyboardEvent) {
    if (event.key !== 'Escape' || event.isComposing || event.keyCode === 229 || (!result() && !running())) return;
    event.preventDefault(); event.stopPropagation();
    if (active) void stop(); else setResult('');
  }
  return <Show when={services().length || running() || result()}><span ref={element} class="extension-space-commands" onKeyDown={keyboard} onPointerDown={event => event.stopPropagation()}>
    <For each={buttons()}>{choice => { const executing = () => running() && extensionServiceKey(running()!) === extensionServiceKey(choice); return <button class="icon-button" type="button" data-plugin-id={choice.record.manifest.id}
      aria-label={executing() ? t('停止插件执行') : choice.service.title} title={executing() ? t('停止') : t(choice.unavailableReason || choice.service.title)}
      disabled={executing() ? stopping() : props.disabled || Boolean(running()) || Boolean(choice.unavailableReason)} onClick={() => executing() ? void stop() : void execute(choice)}>
      <Show when={executing()} fallback={<PluginIcon plugin={choice.record.manifest} sha256={choice.record.sha256} source={choice.record.source} size={18}/>}><Show when={stopping()} fallback={<Square size={15}/>}><LoaderCircle size={17} class="spin"/></Show></Show>
    </button>; }}</For>
    <Show when={result()}><div class="extension-task-popover extension-task-result extension-space-result" role="status"><p>{result()}</p><button type="button" class="icon-button compact" aria-label={t('关闭')} onClick={() => setResult('')}><X size={14}/></button></div></Show>
  </span></Show>;
}
