// SPDX-License-Identifier: MPL-2.0
import { createEffect, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { RefreshCw, Search } from 'lucide-solid';
import { pluginTags, pluginTagLabels } from '../plugin-categories';
import type { PluginTag } from '../plugin-contracts';
import { extensionNative } from '../extension-bridge';
import { runtimeInfo } from '../bridge';
import type { HostPlatform } from '../plugin-platform';
import { SettingsAddButton } from './SettingControls';
import CommunityExtensions from './CommunityExtensions';
import BuiltinPlugins from './BuiltinPlugins';
import { t } from '../i18n';
import '../plugins.css';
import '../extension-ui.css';

export default function PluginsPanel(props: { active: boolean }) {
  const [pane, setPane] = createSignal<'discover' | 'installed' | 'native'>('discover');
  const [query, setQuery] = createSignal(''), [filter, setFilter] = createSignal<PluginTag | ''>('');
  const [communityDetail, setCommunityDetail] = createSignal(false), [builtinDetail, setBuiltinDetail] = createSignal(false);
  const [add, setAdd] = createSignal(0), [refresh, setRefresh] = createSignal(0);
  const [platform, setPlatform] = createSignal<HostPlatform>(), [platformError, setPlatformError] = createSignal('');
  let disposed = false, loadingPlatform = false;
  createEffect(on(() => [props.active, refresh()] as const, ([active]) => {
    if (!active || platform() || loadingPlatform) return;
    loadingPlatform = true; setPlatformError('');
    void runtimeInfo().then(info => { if (!disposed) setPlatform(info.platform); })
      .catch(cause => { if (!disposed) setPlatformError(cause instanceof Error ? cause.message : String(cause)); })
      .finally(() => { loadingPlatform = false; });
  }));
  onCleanup(() => { disposed = true; });
  const detail = () => communityDetail() || builtinDetail();
  return <section class="plugins-panel">
    <header class="settings-page-heading"><h2>{t('插件')}</h2><Show when={!detail() && pane() !== 'native'}><SettingsAddButton disabled={!extensionNative} onClick={() => setAdd(value => value + 1)}/></Show></header>
    <Show when={!detail()}><div class="segmented"><button aria-pressed={pane() === 'discover'} classList={{ selected: pane() === 'discover' }} onClick={() => setPane('discover')}>{t('插件市场')}</button><button aria-pressed={pane() === 'installed'} classList={{ selected: pane() === 'installed' }} onClick={() => setPane('installed')}>{t('已安装')}</button><button aria-pressed={pane() === 'native'} classList={{ selected: pane() === 'native' }} onClick={() => setPane('native')}>{t('原生')}</button></div>
      <div class="plugins-toolbar"><label class="plugins-search"><Search size={14}/><input type="search" aria-label={t('搜索插件')} placeholder={t('搜索插件')} value={query()} onInput={event => setQuery(event.currentTarget.value)}/></label><select aria-label={t('插件类型')} value={filter()} onChange={event => setFilter(event.currentTarget.value as PluginTag | '')}><option value="">{t('全部类型')}</option><For each={pluginTags}>{tag => <option value={tag}>{t(pluginTagLabels[tag])}</option>}</For></select><button class="icon-button" title={t('刷新插件')} aria-label={t('刷新插件')} onClick={() => setRefresh(value => value + 1)}><RefreshCw size={15}/></button></div>
    </Show>
    <Show when={platformError()}><div class="plugin-error" role="alert">{platformError()}</div></Show>
    <Show when={pane() !== 'native'} fallback={<BuiltinPlugins active={props.active} platform={platform()} query={query()} filter={filter()} refreshRequest={refresh()} onDetail={setBuiltinDetail}/>}>
      <CommunityExtensions active={props.active} platform={platform()} pane={pane() === 'installed' ? 'installed' : 'discover'} query={query()} filter={filter()} addRequest={add()} refreshRequest={refresh()} onDetail={setCommunityDetail}/>
    </Show>
  </section>;
}
