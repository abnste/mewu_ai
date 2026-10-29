// SPDX-License-Identifier: MPL-2.0
import { For, Show } from 'solid-js';
import { appearanceState,chooseAppearance } from '../appearance-host';
import { appearanceChoiceKey,readAppearanceSelection } from '../appearance';
import { Row } from './SettingControls';
import { t } from '../i18n';
export default function AppearanceSetting(){
  const selected=()=>appearanceState().selection?JSON.stringify(appearanceState().selection):'';
  return <Row title={t('主题')}><select aria-label={t('主题')} disabled={!appearanceState().ready} value={selected()} onChange={event=>chooseAppearance(readAppearanceSelection(event.currentTarget.value))}>
    <option value="">{t('默认')}</option>
    <Show when={appearanceState().selection&&!appearanceState().active}><option value={selected()} disabled>{t('所选主题不可用')}</option></Show>
    <For each={appearanceState().choices}>{choice=><option value={JSON.stringify({pluginId:choice.pluginId,serviceId:choice.serviceId})} data-theme-key={appearanceChoiceKey(choice)}>{choice.title}</option>}</For>
  </select></Row>;
}
