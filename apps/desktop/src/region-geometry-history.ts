// SPDX-License-Identifier: MPL-2.0
import type { Region, RegionGeometry, RegionGeometryCommand, RegionGeometryEdit, RegionGeometryHistory, ReplayRegionGeometry, Scene, Snapshot } from './contracts';
import { geometryReceipt, geometryTarget, regionGeometry, sameGeometry, type GeometryReceipt } from './region-geometry';

export type GeometryDirection = 'undo' | 'redo';
export const emptyGeometryHistory = (): RegionGeometryHistory => ({ revision: 0, undo: [], redo: [] });
export function geometryHistoryKey(event: { key: string; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; altKey: boolean; repeat: boolean; isComposing: boolean }): GeometryDirection | undefined {
  if (event.repeat || event.isComposing || event.altKey || !(event.ctrlKey || event.metaKey)) return;
  const key = event.key.toLowerCase();
  if (key === 'z') return event.shiftKey ? 'redo' : 'undo';
  if (key === 'y' && !event.shiftKey) return 'redo';
}
export function geometryHistoryCommand(scene: Scene, direction: GeometryDirection): ReplayRegionGeometry | undefined {
  const history = scene.geometryHistory, edit = history?.[direction].at(-1);
  if (!history || !edit) return;
  const target = geometryTarget(scene, edit.regionId), from = direction === 'undo' ? edit.to : edit.from;
  if (!target || target.backgroundId !== edit.backgroundId || target.sourceId !== edit.sourceId || !sameGeometry(target.geometry, from)) throw new Error('区域历史已变化，请重新操作');
  return { type: direction === 'undo' ? 'undo_region_geometry' : 'redo_region_geometry', sceneId: scene.id, expectedHistoryRevision: history.revision, expectedOperationId: edit.id, regionId: edit.regionId, backgroundId: target.backgroundId, sourceId: target.sourceId, expectedRevision: target.revision, from: regionGeometry(target.geometry) };
}
export function geometryHistoryReceipt(snapshot: Snapshot, command: ReplayRegionGeometry, to: RegionGeometry): GeometryReceipt {
  const receipt = geometryReceipt(snapshot, { ...command, type: 'set_region_geometry', to });
  const history = snapshot.scenes.find(value => value.id === command.sceneId)?.geometryHistory;
  const moved = command.type === 'undo_region_geometry' ? history?.redo.at(-1) : history?.undo.at(-1);
  if (history?.revision !== command.expectedHistoryRevision + 1 || moved?.id !== command.expectedOperationId) throw new Error('区域历史已变化，请重新操作');
  return receipt;
}

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const advance = (value: number) => { if (!Number.isSafeInteger(value) || value < 0 || value >= Number.MAX_SAFE_INTEGER) throw new Error('区域版本无效'); return value + 1; };
function historyBytes(history: RegionGeometryHistory): number {
  const entries = [...history.undo, ...history.redo];
  // Reserve u64's 20 digits and one stack's commas. Preview reserves each f64
  // separately because JavaScript and serde may spell the same number differently.
  let bytes = new TextEncoder().encode(JSON.stringify({ revision: 0, undo: entries, redo: [] })).byteLength + 19;
  for (const entry of entries) for (const geometry of [entry.from, entry.to]) for (const value of Object.values(geometry)) bytes += Math.max(0, 32 - JSON.stringify(value).length);
  return bytes;
}
function replaceGeometry(region: Region, to: RegionGeometry, revision: number) {
  Object.assign(region, regionGeometry(to)); region.drawingRevision = revision;
  if (region.imageOverride) { if (region.ocr) region.ocr.drawingRevision = revision; if (region.translation) region.translation.drawingRevision = revision; }
  else { delete region.ocr; delete region.translation; }
}
interface OpenEdit { sceneId: string; editId: string; entryId: string; regionId: string; backgroundId: string; sourceId: string; revision: number }
/** Isolated preview mirror; native Store remains authoritative for desktop. */
export class PreviewGeometryHistory {
  private open?: OpenEdit;
  finish(sceneId: string, editId: string) { if (!uuid.test(editId)) throw new Error('区域编辑标识无效'); if (this.open?.sceneId === sceneId && this.open.editId === editId) this.open = undefined; }
  close() { this.open = undefined; }
  private target(scene: Scene, command: { regionId: string; backgroundId: string; sourceId: string; expectedRevision: number; from: RegionGeometry }) {
    const target = geometryTarget(scene, command.regionId), region = scene.regions.find(value => value.id === command.regionId);
    if (!target || !region || target.backgroundId !== command.backgroundId || target.sourceId !== command.sourceId || target.revision !== command.expectedRevision || !sameGeometry(target.geometry, command.from)) throw new Error('区域位置已变化');
    return { target, region };
  }
  private valid(to: RegionGeometry, width: number, height: number) {
    if ([to.x, to.y, to.width, to.height].some(value => !Number.isFinite(value)) || to.x < 0 || to.y < 0 || to.width <= 0 || to.height <= 0 || to.x + to.width > width || to.y + to.height > height) throw new Error('区域超出截图范围');
  }
  apply(scene: Scene, command: RegionGeometryCommand, id: () => string): boolean {
    if (command.editId !== undefined && !uuid.test(command.editId)) throw new Error('区域编辑标识无效');
    const { target, region } = this.target(scene, command);
    this.valid(command.to, target.width, target.height);
    if (sameGeometry(command.from, command.to)) {
      const open = this.open;
      if (!open || command.editId !== open.editId || scene.id !== open.sceneId || region.id !== open.regionId || target.backgroundId !== open.backgroundId || target.sourceId !== open.sourceId || target.revision !== open.revision) this.open = undefined;
      return false;
    }
    const revision = advance(target.revision), history = structuredClone(scene.geometryHistory ?? emptyGeometryHistory());
    history.revision = advance(history.revision);
    const head = history.undo.at(-1), open = this.open;
    const merge = Boolean(command.editId && open && open.editId === command.editId && open.sceneId === scene.id && open.regionId === region.id && open.backgroundId === target.backgroundId && open.sourceId === target.sourceId && open.revision === target.revision && head?.id === open.entryId && sameGeometry(head.to, command.from) && !history.redo.length);
    let entry: RegionGeometryEdit;
    if (merge) { entry = head!; entry.to = regionGeometry(command.to); }
    else { entry = { id: id(), regionId: region.id, backgroundId: target.backgroundId, sourceId: target.sourceId, from: regionGeometry(command.from), to: regionGeometry(command.to) }; history.undo.push(entry); }
    history.redo = [];
    if (sameGeometry(entry.from, entry.to)) history.undo.pop();
    while (history.undo.length > 50 || historyBytes(history) > 64 * 1024) {
      if (history.undo.length <= 1) throw new Error('几何历史数据过大');
      history.undo.shift();
    }
    replaceGeometry(region, command.to, revision); scene.geometryHistory = history;
    this.open = command.editId && !sameGeometry(entry.from, entry.to) ? { sceneId: scene.id, editId: command.editId, entryId: entry.id, regionId: region.id, backgroundId: target.backgroundId, sourceId: target.sourceId, revision } : undefined;
    return true;
  }
  replay(scene: Scene, command: ReplayRegionGeometry) {
    const history = structuredClone(scene.geometryHistory ?? emptyGeometryHistory()), undo = command.type === 'undo_region_geometry';
    const entry = (undo ? history.undo : history.redo).at(-1);
    if (!entry || history.revision !== command.expectedHistoryRevision || entry.id !== command.expectedOperationId || entry.regionId !== command.regionId || entry.backgroundId !== command.backgroundId || entry.sourceId !== command.sourceId || !sameGeometry(command.from, undo ? entry.to : entry.from)) throw new Error('区域历史已变化，请重新操作');
    const { target, region } = this.target(scene, command), to = undo ? entry.from : entry.to;
    this.valid(to, target.width, target.height);
    const revision = advance(target.revision); history.revision = advance(history.revision);
    (undo ? history.undo : history.redo).pop(); (undo ? history.redo : history.undo).push(entry);
    replaceGeometry(region, to, revision); scene.geometryHistory = history; this.open = undefined;
  }
  invalidate(scene: Scene, regionId?: string) {
    const old = scene.geometryHistory ?? emptyGeometryHistory(), undo = regionId === undefined ? old.undo : old.undo.filter(value => value.regionId !== regionId);
    if (undo.length !== old.undo.length || old.redo.length) scene.geometryHistory = { revision: advance(old.revision), undo, redo: [] };
    this.open = undefined;
  }
}
