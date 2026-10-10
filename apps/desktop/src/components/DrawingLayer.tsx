// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, createUniqueId, For, Match, on, onCleanup, Show, Switch } from 'solid-js';
import type { Asset, Drawing, Region } from '../contracts';
import { getMosaicPreview, type MosaicSource } from '../bridge';
import type { MosaicPreview } from '../mosaic-preview';
import { arrowHead, drawingBounds, drawingOrder } from './drawing-geometry';
import { penSegments } from './drawing-pointer';
import RichDrawingShape from './RichDrawingShape';
import type { RichDrawingContext } from '../drawing-layout-preview';
import './drawing.css';

interface MosaicContext { sceneId: string; background: Asset; override?: MosaicSource }
function MosaicShape(props: { drawing: Drawing; context?: MosaicContext }) {
  const clipId = `mosaic-${createUniqueId()}`;
  const [grid, setGrid] = createSignal<MosaicPreview>();
  const identity = createMemo(() => `${props.context?.sceneId}:${props.context?.background.id}:${props.context?.override?.source.id}:${props.context?.override?.drawingRevision}:${props.drawing.strokeWidth}`);
  createEffect(on(identity, () => {
    setGrid(undefined); let alive = true; const context = props.context;
    if (context) void getMosaicPreview(context.sceneId, context.background, props.drawing.strokeWidth, context.override).then(value => { if (alive) setGrid(value); }).catch(() => { if (alive) setGrid(undefined); });
    onCleanup(() => { alive = false; });
  }));
  const box = () => { const [a, b] = props.drawing.points, x = Math.floor(Math.min(a.x, b.x)), y = Math.floor(Math.min(a.y, b.y)); return { x, y, width: Math.ceil(Math.max(a.x, b.x)) - x, height: Math.ceil(Math.max(a.y, b.y)) - y }; };
  return <g stroke="none"><defs><clipPath id={clipId}><rect {...box()} /></clipPath></defs><rect {...box()} fill="#737d89" /><Show when={grid()}>{value => <image href={value().dataUrl} x="0" y="0" width={value().columns * value().blockSize} height={value().rows * value().blockSize} preserveAspectRatio="none" clip-path={`url(#${clipId})`} style={{ 'image-rendering': 'pixelated' }} onError={() => setGrid(undefined)} />}</Show></g>;
}
export function DrawingShape(props: { drawing: Drawing; interactive?: boolean; selected?: boolean; mosaicContext?: MosaicContext; richContext?: RichDrawingContext; onError?: (message: string) => void }) {
  const segment = penSegments();
  const paths = createMemo(() => props.drawing.kind === 'pen' || props.drawing.kind === 'highlighter' ? segment(props.drawing.points) : []);
  const first = () => props.drawing.points[0] ?? { x: 0, y: 0 };
  const second = () => props.drawing.points[1] ?? first();
  const box = () => ({ x: Math.min(first().x, second().x), y: Math.min(first().y, second().y), width: Math.abs(second().x - first().x), height: Math.abs(second().y - first().y) });
  const bounds = () => drawingBounds(props.drawing);
  const diameter = () => props.drawing.fontSize ?? 28;
  const numberSize = () => Math.min(diameter() * .55, diameter() * .78 / ((props.drawing.text?.length ?? 1) * .6));
  return <g data-drawing-id={props.drawing.id} classList={{ 'drawing-hit': props.interactive }} fill="none" stroke={props.drawing.color} stroke-width={props.drawing.strokeWidth} stroke-linecap="round" stroke-linejoin="round">
    <Switch>
      <Match when={props.drawing.kind === 'rich'}><RichDrawingShape drawing={props.drawing} interactive={props.interactive} context={props.richContext} onError={props.onError} /></Match>
      <Match when={props.drawing.kind === 'pen' || props.drawing.kind === 'highlighter'}><g opacity={props.drawing.kind === 'highlighter' ? .35 : 1}><Show when={props.drawing.points.length > 1} fallback={<circle cx={first().x} cy={first().y} r={props.drawing.strokeWidth / 2} fill={props.drawing.color} stroke="none" />}><For each={paths()}>{value => <path d={value.path} />}</For></Show></g></Match>
      <Match when={props.drawing.kind === 'line'}><line x1={first().x} y1={first().y} x2={second().x} y2={second().y} /></Match>
      <Match when={props.drawing.kind === 'arrow'}><line x1={first().x} y1={first().y} x2={second().x} y2={second().y} /><polygon points={arrowHead(props.drawing.points, props.drawing.strokeWidth)} fill={props.drawing.color} stroke="none" /></Match>
      <Match when={props.drawing.kind === 'rect'}><rect {...box()} /></Match>
      <Match when={props.drawing.kind === 'ellipse'}><ellipse cx={box().x + box().width / 2} cy={box().y + box().height / 2} rx={box().width / 2} ry={box().height / 2} /></Match>
      <Match when={props.drawing.kind === 'mosaic'}><MosaicShape drawing={props.drawing} context={props.mosaicContext} /></Match>
      <Match when={props.drawing.kind === 'number'}><circle cx={first().x + diameter() / 2} cy={first().y + diameter() / 2} r={diameter() / 2} fill={props.drawing.color} stroke="none" /><text x={first().x + diameter() / 2} y={first().y + diameter() / 2 + numberSize() * .35} text-anchor="middle" font-family={'"Segoe UI", "Microsoft YaHei", sans-serif'} font-size={String(numberSize())} font-weight="600" fill="white" stroke="none">{props.drawing.text}</text></Match>
      <Match when={props.drawing.kind === 'text'}><text x={first().x} y={first().y + (props.drawing.fontSize ?? 20)} font-family={'"Segoe UI", "Microsoft YaHei", sans-serif'} font-size={String(props.drawing.fontSize ?? 20)} fill={props.drawing.color} stroke="none" style={{ 'white-space': 'pre' }}><For each={(props.drawing.text ?? '').split('\n')}>{(line, index) => <tspan x={first().x} y={first().y + (props.drawing.fontSize ?? 20) * (1 + index() * 1.25)}>{line || ' '}</tspan>}</For></text></Match>
    </Switch>
    <Show when={props.selected}><rect {...bounds()} fill="none" stroke="#348bf1" stroke-width="1" vector-effect="non-scaling-stroke" stroke-dasharray="4 3" pointer-events="none" /></Show>
  </g>;
}
export default function DrawingLayer(props: { region: Region; sourceRegion?: Region; sceneId?: string; background?: Asset }) {
  const drawings = createMemo(() => new Map((props.region.drawings ?? []).map(value => [value.id, value])));
  const ids = createMemo(() => drawingOrder(props.region.drawings ?? []).map(value => value.id));
  return <Show when={props.region.drawings?.length}><svg class="drawing-layer" viewBox={`${props.region.x} ${props.region.y} ${props.region.width} ${props.region.height}`} preserveAspectRatio="none" aria-hidden="true"><For each={ids()}>{id => <Show when={drawings().get(id)}>{drawing => <DrawingShape drawing={drawing()} richContext={props.sceneId && props.background ? { sceneId: props.sceneId, background: props.background, region: props.sourceRegion ?? props.region } : undefined} mosaicContext={props.sceneId && props.background ? { sceneId: props.sceneId, background: props.background, override: props.region.imageOverride ? { regionId: props.region.id, source: props.region.imageOverride, drawingRevision: props.region.drawingRevision ?? 0 } : undefined } : undefined} />}</Show>}</For></svg></Show>;
}
