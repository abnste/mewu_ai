// SPDX-License-Identifier: MPL-2.0
import type { Drawing, DrawingCommand, DrawingStep, Region } from './contracts';

interface DrawingSessionTarget { sceneId: string; regionId: string; backgroundId: string; sourceId: string }
export type DrawingSession = DrawingSessionTarget & (
  | { kind: 'document' }
  | { kind: 'core' }
);
const edits = (step: DrawingStep) => 'batch' in step ? step.batch : [step];
export function richDocumentHistoryStep(step: DrawingStep | undefined): boolean {
  // Replay preserves the whole saved step, including mixed AI batches.
  // This is document history access, not permission to create new vector objects.
  return Boolean(step && edits(step).length);
}
export function hasDrawingDocument(region: Region | undefined): boolean {
  return Boolean(region && (region.drawings?.some(value => value.kind === 'rich') || region.drawingHistory?.undo.length || region.drawingHistory?.redo.length));
}
export function usesDrawingDocument(region: Region, command: DrawingCommand): boolean {
  if (command.type === 'update_drawing') return command.drawing.kind === 'rich' && region.drawings?.some(value => value.id === command.drawing.id && value.kind === 'rich') === true;
  if (command.type === 'remove_drawing') return region.drawings?.some(value => value.id === command.drawingId && value.kind === 'rich') === true;
  if (command.type === 'undo_drawing') return richDocumentHistoryStep(region.drawingHistory?.undo.at(-1));
  if (command.type === 'redo_drawing') return richDocumentHistoryStep(region.drawingHistory?.redo.at(-1));
  return false;
}
