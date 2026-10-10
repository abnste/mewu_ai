// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, on, onCleanup, Show } from 'solid-js';
import type { Drawing } from '../contracts';
import { getDrawingLayoutPreview } from '../drawing-layout-bridge';
import { DrawingLayoutReader, layoutRequestKey, richPreviewTarget, richSourceIdentity, type DrawingLayoutReadState, type RichDrawingContext } from '../drawing-layout-preview';
export default function RichDrawingShape(props: { drawing: Drawing; interactive?: boolean; context?: RichDrawingContext; onError?: (message: string) => void }) {
  const [state, setState] = createSignal<DrawingLayoutReadState>({ loading: false });
  const reader = new DrawingLayoutReader(getDrawingLayoutPreview, value => { setState(value); if (value.error) props.onError?.(value.error); });
  const request = () => props.context ? richPreviewTarget(props.context, props.drawing) : undefined;
  const source = () => props.context ? richSourceIdentity(props.context) : '';
  const identity = createMemo(() => { const target = request(); return target ? layoutRequestKey(target, source()) : ''; });
  createEffect(on(identity, () => reader.select(request(), source())));
  onCleanup(() => reader.dispose());
  const box = () => { const [a, b] = props.drawing.points; return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.abs(b.x - a.x), height: Math.abs(b.y - a.y) }; };
  // The transparent rectangle makes the whole persisted box selectable, including
  // empty table cells and transparent formula margins; it does not reorder the object.
  return <g stroke="none"><rect {...box()} fill="transparent" pointer-events={props.interactive ? 'all' : 'none'} />
    <Show when={state().preview} keyed fallback={<g pointer-events="none"><rect {...box()} fill="#eef1f5" stroke={state().error ? '#bb666b' : '#dfe5ed'} stroke-width="1" vector-effect="non-scaling-stroke" /><Show when={state().error}><path d={`M${box().x} ${box().y}L${box().x + box().width} ${box().y + box().height}M${box().x + box().width} ${box().y}L${box().x} ${box().y + box().height}`} stroke="#bb666b" stroke-width="1" vector-effect="non-scaling-stroke" /></Show></g>}>{preview => <image href={preview.dataUrl} {...box()} preserveAspectRatio="none" pointer-events="none" onError={() => { if (state().preview === preview) reader.imageFailed(); }} />}</Show>
  </g>;
}
