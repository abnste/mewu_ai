// SPDX-License-Identifier: MPL-2.0
import { batch, createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { LoaderCircle, Puzzle, Square, X } from 'lucide-solid';
import * as api from '../extension-bridge';
import type { ExtensionRecord } from '../extension-contracts';
import { extensionResultText, extensionServiceKey, mergeExtensionRecords, selectionExtensionServices, type ExtensionServiceChoice } from '../extension-state';
import { t } from '../i18n';
import '../extension-ui.css';

interface Props { sceneId: string; regionId: string; draft: string; disabled: boolean; onCountChange: (count: number) => void; onInteractionChange: (active: boolean) => void; onRunningChange: (active: boolean) => void; onError: (message: string) => void }
export default function ExtensionSelectionTools(props: Props) {
  const [records, setRecords] = createSignal<ExtensionRecord[]>([]), [menu, setMenu] = createSignal(false);
  const [taskChoice, setTaskChoice] = createSignal<ExtensionServiceChoice>(), [task, setTask] = createSignal('');
  const [running, setRunning] = createSignal<ExtensionServiceChoice>(), [stopping, setStopping] = createSignal(false);
  const [result, setResult] = createSignal('');
  let disposed = false, generation = 0, eventSerial = 0, unsubscribe: (() => void) | undefined;
  const services = createMemo(() => selectionExtensionServices(records()));
  const accept = (value: ExtensionRecord[]) => { if (!disposed) setRecords(old => mergeExtensionRecords(old, value)); };
  createEffect(() => { props.onCountChange(services().length || running() ? 1 : 0); });
  createEffect(() => props.onInteractionChange(Boolean(menu() || taskChoice() || running() || result())));
  createEffect(() => props.onRunningChange(Boolean(running())));
  createEffect(on(() => [props.sceneId, props.regionId] as const, () => {
    generation++; setMenu(false); setTaskChoice(undefined); setResult('');
    const value = running(); if (value) void api.stopExtension(value.record.manifest.id).catch(() => {});
  }, { defer: true }));
  if (api.extensionNative) void (async () => {
    try {
      unsubscribe = await api.subscribeExtensions(value => { eventSerial++; accept(value); });
      if (disposed) { unsubscribe(); return; }
      const beforeEvents = eventSerial, installed = await api.getExtensions();
      if (beforeEvents === eventSerial) accept(installed);
    } catch (cause) { if (!disposed) props.onError(cause instanceof Error ? cause.message : String(cause)); }
  })();
  onCleanup(() => {
    disposed = true; generation++; unsubscribe?.(); props.onCountChange(0); props.onInteractionChange(false); props.onRunningChange(false);
    const value = running(); if (value) void api.stopExtension(value.record.manifest.id).catch(() => {});
  });
  function choose(choice: ExtensionServiceChoice) {
    if (props.disabled || running()) return;
    if (props.draft.trim()) void execute(choice, props.draft);
    else batch(() => { setTask(''); setTaskChoice(choice); setMenu(false); setResult(''); });
  }
  async function execute(choice: ExtensionServiceChoice, text: string) {
    if (props.disabled || running() || !text.trim()) return;
    const current = generation, sceneId = props.sceneId, regionId = props.regionId;
    const live = services().find(value => extensionServiceKey(value) === extensionServiceKey(choice));
    if (!live || live.record.revision !== choice.record.revision) { props.onError(t('插件状态已变化')); return; }
    batch(() => { setRunning(live); setTaskChoice(undefined); setMenu(false); setResult(''); });
    try {
      const value = await api.invokeExtension({ pluginId: live.record.manifest.id, revision: live.record.revision,
        serviceId: live.service.id, input: { task: text }, sceneId, regionId });
      if (!disposed && generation === current) setResult(extensionResultText(value));
    } catch (cause) { if (!disposed && generation === current) props.onError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (!disposed) { setRunning(undefined); setStopping(false); } }
  }
  async function stop() {
    const value = running(); if (!value || stopping()) return;
    setStopping(true);
    try { await api.stopExtension(value.record.manifest.id); }
    catch (cause) { if (!disposed) { setStopping(false); props.onError(cause instanceof Error ? cause.message : String(cause)); } }
  }
  function keyboard(event: KeyboardEvent) {
    if (event.key === 'Escape' && (taskChoice() || menu() || result())) { event.preventDefault(); event.stopPropagation(); setTaskChoice(undefined); setMenu(false); setResult(''); }
  }
  return <>
    <Show when={services().length || running()}><button class="capture-plugin" data-caption={running() ? t('停止') : services().length === 1 ? services()[0].service.title : t('插件')}
      title={running() ? t('停止') : services().length === 1 ? services()[0].service.title : t('插件')} aria-label={running() ? t('停止插件执行') : t('选区插件')}
      disabled={running() ? stopping() : props.disabled} onClick={() => running() ? void stop() : services().length === 1 ? choose(services()[0]) : setMenu(!menu())}>
      <Show when={running()} fallback={<Puzzle size={18}/>}><Show when={stopping()} fallback={<Square size={16}/>}><LoaderCircle size={18} class="spin"/></Show></Show>
    </button></Show>
    <Show when={menu()}><div class="capture-plugin-menu extension-command-menu" role="menu" onKeyDown={keyboard}><For each={services()}>{choice => <button role="menuitem" disabled={props.disabled} onClick={() => choose(choice)}>{choice.service.title}</button>}</For></div></Show>
    <Show when={taskChoice()}>{choice => <form class="extension-task-popover" role="dialog" aria-label={choice().service.title} onKeyDown={keyboard} onSubmit={event => { event.preventDefault(); void execute(choice(), task()); }}>
      <label class="field-label">{choice().service.title}<textarea autofocus rows={3} aria-label={t('任务')} placeholder={t('输入任务')} value={task()} onInput={event => setTask(event.currentTarget.value)} /></label>
      <div class="extension-task-actions"><button type="button" class="secondary-button" onClick={() => setTaskChoice(undefined)}>{t('取消')}</button><button type="submit" class="primary-button" disabled={props.disabled || !task().trim()}>{t('执行')}</button></div>
    </form>}</Show>
    <Show when={result()}><div class="extension-task-popover extension-task-result" role="status" onKeyDown={keyboard}><p>{result()}</p><button class="icon-button compact" aria-label={t('关闭')} onClick={() => setResult('')}><X size={14}/></button></div></Show>
  </>;
}
