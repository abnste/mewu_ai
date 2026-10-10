// SPDX-License-Identifier: MPL-2.0
import type { Drawing, DrawingCommand, DrawingKind } from '../contracts';

export interface DrawingPropertyDraft {
  mode: 'style' | 'text';
  base?: Drawing;
  drawing: Drawing;
  expectedRevision: number;
  error?: string;
}
export interface PendingDrawingDraft { key: string; draft: DrawingPropertyDraft }
export const strokeKinds: DrawingKind[] = ['pen', 'line', 'arrow', 'rect', 'ellipse', 'highlighter'];
export function copyDrawing(value: Drawing): Drawing { return { ...value, ...(value.origin ? { origin: { ...value.origin } } : {}), ...(value.rich ? { rich: { ...value.rich } } : {}), points: value.points.map(point => ({ ...point })) }; }
export function drawingPreview(drawings: Drawing[], preview?: Drawing): Drawing[] {
  if (!preview) return drawings;
  const index = drawings.findIndex(value => value.id === preview.id);
  if (index < 0) return preview.kind !== 'text' || preview.text?.trim() ? [...drawings, preview] : drawings;
  return drawings.map((value, i) => i === index ? preview : value);
}
export function propertyDraft(value: Drawing, revision: number, mode: 'style' | 'text' = 'style', existing = true): DrawingPropertyDraft {
  if (value.kind === 'rich') throw new Error('此对象不支持修改样式');
  return { mode, base: existing ? copyDrawing(value) : undefined, drawing: copyDrawing(value), expectedRevision: revision };
}
export function changeProperty(value: DrawingPropertyDraft, patch: { color?: string; strokeWidth?: number; fontSize?: number; text?: string }): DrawingPropertyDraft {
  if (value.drawing.kind === 'rich') throw new Error('此对象不支持修改样式');
  const drawing = copyDrawing(value.drawing);
  if (patch.color !== undefined && drawing.kind !== 'mosaic') drawing.color = patch.color;
  if (patch.strokeWidth !== undefined && strokeKinds.includes(drawing.kind)) drawing.strokeWidth = patch.strokeWidth;
  if (patch.fontSize !== undefined && ['text', 'number'].includes(drawing.kind)) drawing.fontSize = patch.fontSize;
  if (patch.text !== undefined && value.mode === 'text' && drawing.kind === 'text') drawing.text = patch.text;
  return { ...value, drawing, error: undefined };
}
export function draftChanged(value: DrawingPropertyDraft): boolean {
  if (!value.base) return Boolean(value.drawing.text?.trim());
  return JSON.stringify(value.base) !== JSON.stringify(value.drawing);
}
export function drawingPropertyCommand(value: DrawingPropertyDraft, target: { sceneId: string; backgroundId: string; regionId: string }): DrawingCommand | undefined {
  if (value.drawing.kind === 'rich') throw new Error('此对象不支持修改样式');
  if (!draftChanged(value)) return undefined;
  const drawing = copyDrawing(value.drawing);
  if (drawing.kind === 'text') {
    if (!drawing.text?.trim()) throw new Error('文字不能为空');
    if ([...drawing.text].length > 2000) throw new Error('文字最多 2000 字');
    if ([...drawing.text].some(character => (character.codePointAt(0)! < 32 && character !== '\n') || character === '\x7f')) throw new Error('文字包含不支持的控制字符');
  }
  return { ...target, expectedRevision: value.expectedRevision, type: value.base ? 'update_drawing' : 'add_drawing', drawing };
}

// Unsaved drafts live only in this renderer runtime. Never silently evict user input.
export class DrawingDraftCache {
  private entries = new Map<string, DrawingPropertyDraft>();
  constructor(private limit = 32, private maxBytes = 4 * 1024 * 1024) {}
  get(key: string): DrawingPropertyDraft | undefined { return this.entries.get(key); }
  hasPending(sceneId?: string): boolean {
    return this.list(sceneId).length > 0;
  }
  list(sceneId?: string): PendingDrawingDraft[] {
    return [...this.entries].filter(([key]) => {
      if (sceneId === undefined) return true;
      try { return JSON.parse(key)[0] === sceneId; } catch { return false; }
    }).map(([key, draft]) => ({ key, draft }));
  }
  remember(key: string, value: DrawingPropertyDraft): void {
    if (draftChanged(value)) this.set(key, value); else this.entries.delete(key);
  }
  set(key: string, value: DrawingPropertyDraft): void {
    if (!this.entries.has(key) && this.entries.size >= this.limit) throw new Error('未保存标注过多，请先保存或取消');
    const candidates = [...this.entries].filter(([id]) => id !== key); candidates.push([key, value]);
    if (new TextEncoder().encode(JSON.stringify(candidates)).length > this.maxBytes) throw new Error('未保存标注过多，请先保存或取消');
    this.entries.set(key, value);
  }
  delete(key: string, expected?: DrawingPropertyDraft): boolean {
    if (!expected || this.entries.get(key) === expected) return this.entries.delete(key);
    return false;
  }
}
export const drawingDrafts = new DrawingDraftCache();
export const hasUnsavedDrawingDrafts = (sceneId?: string) => drawingDrafts.hasPending(sceneId);
export const listUnsavedDrawingDrafts = (sceneId?: string) => drawingDrafts.list(sceneId);
export const discardUnsavedDrawingDraft = (entry: PendingDrawingDraft) => drawingDrafts.delete(entry.key, entry.draft);
export function drawingDraftKey(target: { sceneId: string; backgroundId: string; regionId: string; sourceId: string; x: number; y: number; width: number; height: number }): string {
  return JSON.stringify([target.sceneId, target.backgroundId, target.regionId, target.sourceId, target.x, target.y, target.width, target.height]);
}

// Identity is checked after the real promise settles; canceled UI does not release
// the command slot early or run a continuation against another editor.
export async function settleDrawingEdit<T>(request: () => Promise<T>, current: () => boolean, persisted: () => void = () => {}): Promise<boolean> {
  await request(); persisted(); return current();
}
