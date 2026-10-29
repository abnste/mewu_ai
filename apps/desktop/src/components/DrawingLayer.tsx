// SPDX-License-Identifier: MPL-2.0
import { createEffect, createMemo, createSignal, createUniqueId, For, Match, on, onCleanup, Show, Switch } from 'solid-js';
import type { Asset, Drawing, Region } from '../contracts';
import { getMosaicPreview, getHighlightPreview, type MosaicSource } from '../bridge';
import type { MosaicPreview } from '../mosaic-preview';
import { arrowHead, drawingBounds, drawingOrder, drawingTextLayout, drawingTextLineTransform, markPaths } from './drawing-geometry';
import { penSegments } from './drawing-pointer';
import RichDrawingShape from './RichDrawingShape';
import type { RichDrawingContext } from '../drawing-layout-preview';
import './drawing.css';
import { numberForeground, highlightMaskTable } from '../drawing-colors';
import type { HighlightPreview } from '../highlight-preview';
import { highlightSourceIdentity } from '../highlight-source';

interface MosaicContext { sceneId: string; background: Asset; override?: MosaicSource; region?: Region }
function HighlighterShape(props: { drawing: Drawing; interactive?: boolean; context?: MosaicContext; onError?: (message: string) => void }) {
  const id = `highlight-${createUniqueId()}`, segment = penSegments();
  const [foreground, setForeground] = createSignal<HighlightPreview>();
  const identity = createMemo(() => props.context ? highlightSourceIdentity(props.context.sceneId, props.context.background, props.context.region) : '');
  createEffect(on(identity, () => {
    setForeground(undefined); let alive = true; const context = props.context;
    if (context) void getHighlightPreview(context.sceneId, context.background, context.region).then(value => { if (alive) setForeground(value); }).catch(error => { if (alive) props.onError?.(error instanceof Error ? error.message : '无法生成荧光笔预览'); });
    onCleanup(() => { alive = false; });
  }));
  const paths = createMemo(() => segment(props.drawing.points));
  const bounds = () => drawingBounds(props.drawing);
  const mark = createMemo(() => props.drawing.kind === 'mark' ? markPaths(props.drawing) : undefined);
  const ink = () => <Show when={mark()} fallback={<Show when={props.drawing.points.length > 1} fallback={<circle cx={props.drawing.points[0]?.x ?? 0} cy={props.drawing.points[0]?.y ?? 0} r={props.drawing.strokeWidth / 2} fill={props.drawing.color} stroke="none" />}><For each={paths()}>{value => <path d={value.path} />}</For></Show>}>{value => <g fill={props.drawing.color} stroke="none"><path d={value().border} fill-rule="evenodd" fill-opacity={190 / 255} /><path d={value().fill} fill-opacity={84 / 255} /></g>}</Show>;
  return <Show when={props.context} fallback={<g opacity={mark() ? 1 : .35}>{ink()}</g>}>
    <Show when={foreground()}>{value => <g opacity={mark() ? 1 : .35} stroke="none" pointer-events="none">
      <defs>
        <mask id={`${id}-ink`} maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse" {...bounds()} style={{ 'mask-type': 'alpha' }}><g fill="none" stroke="white" stroke-width={props.drawing.strokeWidth} stroke-linecap="round" stroke-linejoin="round">{ink()}</g></mask>
        <filter id={`${id}-coverage`} filterUnits="userSpaceOnUse" {...bounds()} color-interpolation-filters="sRGB"><feComponentTransfer><feFuncA type="discrete" tableValues={highlightMaskTable} /></feComponentTransfer></filter>
        <mask id={`${id}-mask`} maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse" {...bounds()} style={{ 'mask-type': 'alpha' }}><image href={value().dataUrl} x={value().x} y={value().y} width={value().width} height={value().height} filter={`url(#${id}-coverage)`} /></mask>
      </defs>
      <g mask={`url(#${id}-ink)`}><g mask={`url(#${id}-mask)`}><rect x={value().x} y={value().y} width={value().width} height={value().height} fill={props.drawing.color} /><image href={value().dataUrl} x={value().x} y={value().y} width={value().width} height={value().height} onError={() => { setForeground(undefined); props.onError?.('无法生成荧光笔预览'); }} /></g></g>
    </g>}</Show>
    <Show when={props.interactive}><g opacity={0} fill="none" stroke={props.drawing.color} pointer-events="stroke"><Show when={mark()} fallback={<Show when={props.drawing.points.length > 1} fallback={<circle cx={props.drawing.points[0]?.x ?? 0} cy={props.drawing.points[0]?.y ?? 0} r={props.drawing.strokeWidth / 2} fill={props.drawing.color} stroke="none" pointer-events="fill" />}><For each={paths()}>{value => <path d={value.path} />}</For></Show>}>{value => <path d={value().outer} fill={props.drawing.color} stroke="none" pointer-events="fill" />}</Show></g></Show>
  </Show>;
}
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
  const textLayout = createMemo(() => props.drawing.kind === 'text' ? drawingTextLayout(props.drawing) : undefined);
  return <g data-drawing-id={props.drawing.id} classList={{ 'drawing-hit': props.interactive }} fill="none" stroke={props.drawing.color} stroke-width={props.drawing.strokeWidth} stroke-linecap="round" stroke-linejoin="round">
    <Switch>
      <Match when={props.drawing.kind === 'rich'}><RichDrawingShape drawing={props.drawing} interactive={props.interactive} context={props.richContext} onError={props.onError} /></Match>
      <Match when={props.drawing.kind === 'pen'}><g><Show when={props.drawing.points.length > 1} fallback={<circle cx={first().x} cy={first().y} r={props.drawing.strokeWidth / 2} fill={props.drawing.color} stroke="none" />}><For each={paths()}>{value => <path d={value.path} />}</For></Show></g></Match>
      <Match when={props.drawing.kind === 'highlighter' || props.drawing.kind === 'mark'}><HighlighterShape drawing={props.drawing} interactive={props.interactive} context={props.mosaicContext} onError={props.onError} /></Match>
      <Match when={props.drawing.kind === 'line'}><line x1={first().x} y1={first().y} x2={second().x} y2={second().y} /></Match>
      <Match when={props.drawing.kind === 'arrow'}><line x1={first().x} y1={first().y} x2={second().x} y2={second().y} /><polyline points={arrowHead(props.drawing.points)} fill="none" /></Match>
      <Match when={props.drawing.kind === 'rect'}><rect {...box()} /></Match>
      <Match when={props.drawing.kind === 'ellipse'}><ellipse cx={box().x + box().width / 2} cy={box().y + box().height / 2} rx={box().width / 2} ry={box().height / 2} /></Match>
      <Match when={props.drawing.kind === 'mosaic'}><MosaicShape drawing={props.drawing} context={props.mosaicContext} /></Match>
      <Match when={props.drawing.kind === 'number'}><circle cx={first().x + diameter() / 2} cy={first().y + diameter() / 2} r={diameter() / 2} fill={props.drawing.color} stroke="none" /><text x={first().x + diameter() / 2} y={first().y + diameter() / 2 + numberSize() * .35} text-anchor="middle" font-family={'"Segoe UI", "Microsoft YaHei", sans-serif'} font-size={String(numberSize())} font-weight="600" fill={numberForeground(props.drawing.color)} stroke="none">{props.drawing.text}</text></Match>
      <Match when={props.drawing.kind === 'text'}><g font-family={'"Segoe UI", "Microsoft YaHei", sans-serif'} font-size={String(props.drawing.fontSize ?? 20)} font-style="normal" stroke="none" style={{ 'white-space': 'pre' }}><For each={textLayout()?.styledLines ?? []}>{(line,index)=><For each={line}>{run=><text x={first().x+run.x} y={first().y+(props.drawing.fontSize??20)*(1+index()*1.25)} fill={run.color} font-weight={run.textStyle?.bold?'700':'400'} transform={drawingTextLineTransform(run.textStyle?.italic,first().y+(props.drawing.fontSize??20)*(1+index()*1.25))} style={{'text-decoration':[run.textStyle?.underline&&'underline',run.textStyle?.strikethrough&&'line-through'].filter(Boolean).join(' ')||'none'}}>{run.text||' '}</text>}</For>}</For></g></Match>
    </Switch>
    <Show when={props.selected}><rect {...bounds()} fill="none" stroke="#348bf1" stroke-width="1" vector-effect="non-scaling-stroke" stroke-dasharray="4 3" pointer-events="none" /></Show>
  </g>;
}
export default function DrawingLayer(props: { region: Region; sourceRegion?: Region; sceneId?: string; background?: Asset; hidden?: boolean; imageItemId?: string }) {
  const drawings = createMemo(() => new Map((props.region.drawings ?? []).map(value => [value.id, value])));
  const ids = createMemo(() => drawingOrder(props.region.drawings ?? []).map(value => value.id));
  return <Show when={props.region.drawings?.length}><svg class="drawing-layer" style={{ visibility: props.hidden ? 'hidden' : 'visible' }} viewBox={`${props.region.x} ${props.region.y} ${props.region.width} ${props.region.height}`} preserveAspectRatio="none" aria-hidden="true"><For each={ids()}>{id => <Show when={drawings().get(id)}>{drawing => <DrawingShape drawing={drawing()} richContext={props.sceneId && props.background ? { sceneId: props.sceneId, background: props.background, region: props.sourceRegion ?? props.region, imageItemId: props.imageItemId } : undefined} mosaicContext={!props.imageItemId && props.sceneId && props.background ? { sceneId: props.sceneId, background: props.background, region: props.sourceRegion ?? props.region, override: props.region.imageOverride ? { regionId: props.region.id, source: props.region.imageOverride, drawingRevision: props.region.drawingRevision ?? 0 } : undefined } : undefined} />}</Show>}</For></svg></Show>;
}
