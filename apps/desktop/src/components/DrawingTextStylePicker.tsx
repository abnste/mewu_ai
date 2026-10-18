// SPDX-License-Identifier: MPL-2.0
import { For } from 'solid-js';
import { Bold, Italic, Strikethrough, Underline } from 'lucide-solid';
import type { DrawingTextStyle } from '../contracts';
import { t } from '../i18n';

const styles = [
  { key: 'bold', label: '加粗', Icon: Bold },
  { key: 'italic', label: '斜体', Icon: Italic },
  { key: 'underline', label: '下划线', Icon: Underline },
  { key: 'strikethrough', label: '删除线', Icon: Strikethrough },
] as const;

export default function DrawingTextStylePicker(props: {
  value?: DrawingTextStyle;
  disabled: boolean;
  preserveFocus?: boolean;
  onChange: (value: DrawingTextStyle) => void;
}) {
  return <For each={styles}>{style => <button
    title={t(style.label)} aria-label={t(style.label)} data-caption={t(style.label)}
    aria-pressed={!!props.value?.[style.key]} classList={{ selected: !!props.value?.[style.key] }}
    disabled={props.disabled}
    onPointerDown={event => { if (props.preserveFocus && event.button === 0) event.preventDefault(); }}
    onClick={() => {
      if (props.disabled) return;
      const value = { ...props.value };
      if (value[style.key]) delete value[style.key]; else value[style.key] = true;
      props.onChange(value);
    }}
  ><style.Icon size={18} /></button>}</For>;
}
