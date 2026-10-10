// SPDX-License-Identifier: MPL-2.0
import { t } from "../i18n";
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { Check, ChevronLeft, ChevronRight, LoaderCircle, Pencil, Plus, Search, Trash2, X } from 'lucide-solid';
import type { MemoryEntry, MemoryPage, MemoryStats, Scene, SceneCommand } from '../contracts';
import { memoryEntry, memoryPage } from '../bridge';

export type SaveMemory = Omit<Extract<SceneCommand, { type: 'save_memory' }>, 'type'>;
export type DeleteMemory = Omit<Extract<SceneCommand, { type: 'delete_memory' }>, 'type'>;
interface Props {
  agentId: string; stats?: MemoryStats; scenes: Scene[]; savedAgent: boolean; active: boolean;
  onSave: (value: SaveMemory) => Promise<void>;
  onDelete: (value: DeleteMemory) => Promise<void>;
}
interface Draft { id?: string; text: string; revision?: number; error?: string }
const count = (text: string) => [...text].length;
const origins = { legacy_memory: '原有记忆', legacy_reference: '原有参考资料', manual: '手动保存', conversation: '对话中保存' };

export default function MemoryList(props: Props) {
  // Page refreshes never replace drafts. A write uses the revision the user saw.
  const [drafts, setDrafts] = createSignal<Record<string, Draft>>({});
  const [selected, setSelected] = createSignal<Record<string, string>>({});
  const [pending, setPending] = createSignal<Record<string, boolean>>({});
  const [queries, setQueries] = createSignal<Record<string, string>>({});
  const [expanded, setExpanded] = createSignal(false);
  const [page, setPage] = createSignal<MemoryPage>();
  const [pageIndex, setPageIndex] = createSignal(0);
  const [cursors, setCursors] = createSignal<(string | undefined)[]>([undefined]);
  const [loading, setLoading] = createSignal(false);
  const [loadError, setLoadError] = createSignal('');
  const agentId = createMemo(() => props.agentId);
  const query = createMemo(() => (queries()[agentId()] ?? '').trim());
  const libraryRevision = createMemo(() => props.stats?.revision ?? 0);
  const visible = createMemo(() => props.active && expanded() && props.savedAgent);
  const key = (agentId: string, id: string) => `${agentId}/${id}`;
  const currentKey = () => selected()[props.agentId] ? key(props.agentId, selected()[props.agentId]) : '';
  const draft = () => drafts()[currentKey()];
  const entries = () => page()?.entries ?? [];
  const entry = (id: string) => entries().find(item => item.id === id);
  let requestSerial = 0, disposed = false, queryTimer: ReturnType<typeof setTimeout> | undefined;

  async function load(cursor?: string, destination = 0) {
    const agentId = props.agentId, search = query(), serial = ++requestSerial;
    setLoading(true); setLoadError('');
    try {
      const result = await memoryPage({ agentId, query: search, cursor, limit: 25 });
      if (disposed || serial !== requestSerial) return;
      setPage(result); setPageIndex(destination);
      setCursors(previous => [...previous.slice(0, destination), cursor]);
    } catch (error) {
      if (!disposed && serial === requestSerial) setLoadError(error instanceof Error ? error.message : String(error));
    } finally { if (!disposed && serial === requestSerial) setLoading(false); }
  }
  function refresh() {
    if (queryTimer) clearTimeout(queryTimer);
    requestSerial++; setPage(undefined); setCursors([undefined]); setPageIndex(0);
    if (visible()) void load();
  }
  createEffect(on([agentId, query, libraryRevision, visible], (_, previous) => {
    if (queryTimer) clearTimeout(queryTimer);
    requestSerial++; setPage(undefined); setCursors([undefined]); setPageIndex(0); setLoadError(''); setLoading(false);
    if (!visible()) return;
    const queryChanged = previous && previous[0] === props.agentId && previous[1] !== query();
    if (queryChanged) { setLoading(true); queryTimer = setTimeout(() => { queryTimer = undefined; void load(); }, 180); }
    else void load();
  }));
  onCleanup(() => { disposed = true; requestSerial++; if (queryTimer) clearTimeout(queryTimer); });

  const clearDraft = (agentId: string, id: string, saved?: Draft) => {
    const target = key(agentId, id);
    if (saved && drafts()[target] !== saved) return;
    setDrafts(old => { const next = { ...old }; delete next[target]; return next; });
    setSelected(old => { if (old[agentId] !== id) return old; const next = { ...old }; delete next[agentId]; return next; });
  };
  function edit(value?: MemoryEntry) {
    const id = value?.id ?? 'new', target = key(props.agentId, id);
    if (!drafts()[target]) setDrafts(old => ({ ...old, [target]: { id: value?.id, text: value?.text ?? '', revision: value?.revision } }));
    setSelected(old => ({ ...old, [props.agentId]: id }));
  }
  function update(text: string) {
    const target = currentKey();
    setDrafts(old => ({ ...old, [target]: { ...old[target], text, error: undefined } }));
  }
  async function save() {
    const agentId = props.agentId, id = selected()[agentId], target = key(agentId, id), value = drafts()[target];
    const previousRevision = libraryRevision();
    if (!value || pending()[target]) return;
    const text = value.text.trim();
    const error = !text ? '请输入记忆内容' : count(text) > 2000 ? '每条记忆最多 2000 字' : '';
    if (error) { setDrafts(old => ({ ...old, [target]: { ...value, error } })); return; }
    setPending(old => ({ ...old, [target]: true }));
    try {
      await props.onSave({ agentId, id: value.id, text, expectedRevision: value.revision });
      if (disposed) return;
      clearDraft(agentId, id, value);
      // Also refresh idempotent creates, whose stats revision does not change.
      if (agentId === props.agentId && libraryRevision() === previousRevision) refresh();
    } catch (error) {
      if (!disposed) setDrafts(old => ({ ...old, [target]: { ...(old[target] ?? value), error: error instanceof Error ? error.message : String(error) } }));
    } finally { if (!disposed) setPending(old => ({ ...old, [target]: false })); }
  }
  async function remove(value: MemoryEntry) {
    const agentId = props.agentId, target = key(agentId, value.id);
    const previousRevision = libraryRevision();
    if (pending()[target]) return;
    setPending(old => ({ ...old, [target]: true }));
    try {
      await props.onDelete({ agentId, id: value.id, expectedRevision: value.revision });
      if (disposed) return;
      clearDraft(agentId, value.id);
      if (agentId === props.agentId && libraryRevision() === previousRevision) refresh();
    } catch (error) {
      if (disposed) return;
      setDrafts(old => ({ ...old, [target]: { ...(old[target] ?? { id: value.id, text: value.text, revision: value.revision }), error: error instanceof Error ? error.message : String(error) } }));
      setSelected(old => ({ ...old, [agentId]: value.id }));
    } finally { if (!disposed) setPending(old => ({ ...old, [target]: false })); }
  }
  async function reloadDraft() {
    const agentId = props.agentId, target = currentKey(), value = drafts()[target];
    if (!value?.id || pending()[target]) return;
    setPending(old => ({ ...old, [target]: true }));
    try {
      const latest = await memoryEntry({ agentId, id: value.id });
      if (disposed || props.agentId !== agentId || currentKey() !== target || drafts()[target] !== value) return;
      setDrafts(old => ({ ...old, [target]: latest ? { id: latest.id, text: latest.text, revision: latest.revision } : { ...value, error: '记忆已删除，当前草稿已保留' } }));
    } catch (error) {
      if (!disposed && drafts()[target] === value) setDrafts(old => ({ ...old, [target]: { ...value, error: error instanceof Error ? error.message : String(error) } }));
    } finally { if (!disposed) setPending(old => ({ ...old, [target]: false })); }
  }
  function sourceText(value: MemoryEntry) {
    const source = value.source, scene = props.scenes.find(scene => scene.id === source?.sceneId);
    const message = scene?.messages.find(message => message.id === source?.messageId);
    return { title: scene?.title ?? t('会话已不可用'), text: message?.text ?? '', time: new Date(message?.createdAt ?? value.updatedAt).toLocaleString() };
  }
  return <section class="memory-section" aria-label={t("已保存的记忆")}>
    <header class="memory-heading"><button class="memory-expand text-button" aria-expanded={expanded()} disabled={!props.savedAgent} title={props.savedAgent ? undefined : t('先保存 Agent')} onClick={() => setExpanded(!expanded())}><ChevronRight size={14} classList={{ expanded: expanded() }} />{t("本地记忆")}</button><span>{props.stats?.count ?? 0}{t("条")}</span></header>
    <div hidden={!expanded()}>
      <div class="memory-browser-controls"><label class="memory-search"><Search size={14} /><input type="search" aria-label={t("搜索记忆")} placeholder={t("搜索记忆")} value={queries()[props.agentId] ?? ''} onInput={event => { const value = event.currentTarget.value; setQueries(old => ({ ...old, [props.agentId]: value })); }} /></label><button class="secondary-button" disabled={!props.savedAgent} onClick={() => edit()}><Plus size={13} />{t("添加")}</button></div>
      <Show when={draft()}>{_value => <div class="memory-editor">
        <label class="field-label">{draft()!.id ? t('编辑记忆') : t('添加记忆')}<textarea aria-label={draft()!.id ? t('编辑记忆') : t('添加记忆')} rows={3} disabled={pending()[currentKey()]} value={draft()!.text} onInput={event => update(event.currentTarget.value)} /></label>
        <div class="memory-editor-actions"><span classList={{ 'over-limit': count(draft()!.text.trim()) > 2000 }}>{count(draft()!.text.trim())}/2000</span><Show when={draft()?.id}><button class="text-button" disabled={pending()[currentKey()]} onClick={() => void reloadDraft()}>{t("载入最新")}</button></Show><button class="secondary-button" disabled={pending()[currentKey()]} onClick={() => clearDraft(props.agentId, selected()[props.agentId])}><X size={12} />{t("取消")}</button><button class="primary-button" disabled={pending()[currentKey()]} onClick={() => void save()}><Show when={pending()[currentKey()]} fallback={<Check size={12} />}><LoaderCircle size={12} class="spin" /></Show>{t("保存")}</button></div>
        <Show when={draft()?.error}><p class="settings-error" role="alert">{draft()!.error}</p></Show>
      </div>}</Show>
      <Show when={loadError()}><div class="memory-load-error" role="alert"><p class="settings-error">{loadError()}</p><button class="text-button" onClick={refresh}>{t("重新查看")}</button></div></Show>
      <div class="memory-list" aria-busy={loading()}><For each={entries().map(value => value.id)}>{id => <Show when={entry(id)}>{value => <article class="memory-entry">
        <div class="memory-entry-row"><p>{value().text}</p><div class="memory-entry-actions"><button class="icon-button compact" title={t("编辑记忆")} aria-label={t("编辑记忆")} disabled={pending()[key(props.agentId, id)]} onClick={() => edit(value())}><Pencil size={13} /></button><button class="icon-button compact" title={t("删除记忆")} aria-label={t("删除记忆")} disabled={pending()[key(props.agentId, id)]} onClick={() => void remove(value())}><Trash2 size={13} /></button></div></div>
        <Show when={value().source} fallback={<Show when={value().origin}><span class="memory-origin">{t(origins[value().origin!])}</span></Show>}><details class="memory-source"><summary><ChevronRight size={12} />{t("来源")}</summary><span>{sourceText(value()).title} · {sourceText(value()).time}</span><Show when={sourceText(value()).text}><p>{sourceText(value()).text}</p></Show></details></Show>
      </article>}</Show>}</For></div>
      <footer class="memory-pagination"><span role="status"><Show when={loading()} fallback={page() ? t("{0}第 {1} 页").replaceAll("{0}", () => String(query() ? t("{0} 条 · ").replaceAll("{0}", () => String(page()!.total)) : '')).replaceAll("{1}", () => String(pageIndex() + 1)) : ''}><LoaderCircle size={13} class="spin" />{t("加载中")}</Show></span><button class="icon-button compact" title={t("上一页")} aria-label={t("上一页记忆")} disabled={loading() || pageIndex() === 0} onClick={() => void load(cursors()[pageIndex() - 1], pageIndex() - 1)}><ChevronLeft size={16} /></button><button class="icon-button compact" title={t("下一页")} aria-label={t("下一页记忆")} disabled={loading() || !page()?.nextCursor} onClick={() => void load(page()!.nextCursor, pageIndex() + 1)}><ChevronRight size={16} /></button></footer>
    </div>
  </section>;
}
