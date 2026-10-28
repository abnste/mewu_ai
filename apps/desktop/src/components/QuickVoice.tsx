// SPDX-License-Identifier: MPL-2.0
import { Show } from 'solid-js';
import { t } from '../i18n';
import VoiceInputButton,{type VoiceControl} from './VoiceInputButton';
import Markdown from './Markdown';
import './quick-interaction.css';
export default function QuickVoice(props:{voice?:VoiceControl;ready?:boolean;text?:string;draft?:string;error?:string;thinking?:boolean}){
  return <aside class="quick-voice" aria-label={t("快速交互")}>
    <Show when={props.text}><div class="quick-voice-answer"><Markdown text={props.text!}/></div></Show>
    <Show when={props.draft&&!props.voice?.pending}><div class="quick-voice-transcript">{props.draft}</div></Show>
    <Show when={props.error}><div class="quick-voice-error" role="status">{props.error}</div></Show>
    <Show when={props.voice}>{voice=><div class="quick-voice-bubble" classList={{listening:voice().pending?.phase==='listening',thinking:props.thinking}}><VoiceInputButton {...voice()} disabled={voice().disabled||props.ready===false} onMenu={()=>{}}/></div>}</Show>
  </aside>;
}
