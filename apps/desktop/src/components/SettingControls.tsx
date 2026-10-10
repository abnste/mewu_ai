// SPDX-License-Identifier: MPL-2.0
import { Show, type JSX } from 'solid-js';
import { Plus } from 'lucide-solid';
import { t } from '../i18n';

export function SettingsAddButton(props: { disabled?: boolean; title?: string; onClick: () => void }) {
  return <button type="button" class="secondary-button settings-add-button" disabled={props.disabled} title={props.title} onClick={props.onClick}><Plus size={14} aria-hidden="true" /><span>{t('添加')}</span></button>;
}

export function Row(props: { title: string; children: JSX.Element; caption?: string }) {
  return <div class="setting-row"><div class="setting-label"><span>{props.title}</span><Show when={props.caption}><small>{props.caption}</small></Show></div><div class="setting-control">{props.children}</div></div>;
}
export function Toggle(props: { checked: boolean; label: string; disabled?: boolean; onChange: (value: boolean) => void }) {
  return <button class="toggle" classList={{ checked: props.checked }} role="switch" aria-checked={props.checked} aria-label={props.label} disabled={props.disabled} onClick={() => props.onChange(!props.checked)}><span /></button>;
}
