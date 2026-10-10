// SPDX-License-Identifier: MPL-2.0
import { createMemo } from 'solid-js';
import type { Drawing } from '../contracts';
import { t } from '../i18n';

/** Native caret/selection over the actual SVG text, never a second text preview. */
export default function InlineDrawingText(props: {
  drawing: Drawing; disabled: boolean;
  ref: (element: HTMLTextAreaElement) => void;
  onInput: (text: string) => void; onConfirm: () => void; onBlur: (event: FocusEvent) => void;
}) {
  const context = document.createElement('canvas').getContext('2d')!;
  const layout = createMemo(() => {
    const size = props.drawing.fontSize ?? 20, lines = (props.drawing.text ?? '').split('\n');
    context.font = `${size}px "Segoe UI", "Microsoft YaHei", sans-serif`;
    const metrics = context.measureText('Mg中'), lineHeight = size * 1.25;
    const ascent = metrics.fontBoundingBoxAscent ?? size * .8;
    const descent = metrics.fontBoundingBoxDescent ?? size * .2;
    const baseline = ascent + (lineHeight - ascent - descent) / 2;
    return {
      x: props.drawing.points[0].x, y: props.drawing.points[0].y + size - baseline,
      width: Math.max(size * 2, ...lines.map(line => context.measureText(line).width + size)),
      height: lines.length * lineHeight, size, lineHeight,
    };
  });
  return <foreignObject class="drawing-text-editor" x={layout().x} y={layout().y} width={layout().width} height={layout().height}>
    <textarea ref={props.ref} aria-label={t('绘制文字')} wrap="off" spellcheck={false}
      value={props.drawing.text ?? ''} disabled={props.disabled}
      style={{ 'font-size': `${layout().size}px`, 'line-height': `${layout().lineHeight}px`, 'caret-color': props.drawing.color }}
      onPointerDown={event => event.stopPropagation()} onContextMenu={event => event.preventDefault()}
      onInput={event => props.onInput(event.currentTarget.value)} onBlur={props.onBlur}
      onKeyDown={event => {
        if (!event.isComposing && event.keyCode !== 229 && event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
          event.preventDefault(); event.stopPropagation(); props.onConfirm();
        }
      }} />
  </foreignObject>;
}
