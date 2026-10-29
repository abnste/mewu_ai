// SPDX-License-Identifier: MPL-2.0
import type { ImageExportVariant, SceneCommand, SpaceItem } from '../contracts';
import type { DrawingEditorPort, EditorBox, EditorDrawingAction, RegisterDrawingHistory } from '../drawing-editor-port';
import type { RegisterDrawingFlush } from '../drawing-flush';
import { imageAnnotationRegion } from '../image-annotations';
import { coreDrawingTools } from '../core-drawing';
import { drawingPropertyCommand } from './drawing-properties';
import { richPreviewTarget, richSourceIdentity, type DrawingLayoutTarget, type DrawingTableFormat } from '../drawing-layout-preview';
import SharedDrawingEditor from './SharedDrawingEditor';
import { DrawingShape } from './DrawingLayer';
interface Props {
  sceneId: string; item: SpaceItem; box: EditorBox; busy: boolean; inputLocked?: boolean;
  onCommand: (command: SceneCommand) => Promise<void>; onClose: () => void; onError: (message: string) => void;
  onExport: (copy: boolean, closeSpace: boolean, variant?: ImageExportVariant) => Promise<boolean>; onPin?: () => Promise<boolean>;
  onRegisterFlush?: RegisterDrawingFlush; onRegisterHistory?: RegisterDrawingHistory;
  onCopyTable: (target: DrawingLayoutTarget, format: DrawingTableFormat) => Promise<void>;
}
export default function ImageDrawingEditor(props: Props) {
  const region = () => imageAnnotationRegion(props.item);
  const context = () => ({ sceneId: props.sceneId, background: props.item.asset, region: region(), imageItemId: props.item.id });
  const port: DrawingEditorPort = {
    pixelEraser: true,
    frame: () => ({ x: 0, y: 0, width: region().width, height: region().height }),
    sourceSize: () => ({ width: region().width, height: region().height }),
    sourceIdentity: () => richSourceIdentity(context()),
    draftScope: () => props.sceneId,
    draftKey: () => JSON.stringify(['image-item', props.sceneId, props.item.id, props.item.asset.id]),
    revision: () => region().drawingRevision ?? 0, drawings: () => region().drawings ?? [],
    canSelect: drawing => drawing.rich?.kind !== 'repair', canStyle: drawing => drawing.kind !== 'rich',
    allows: action => action.type !== 'move_stored',
    propertyCommand: draft => {
      const value = drawingPropertyCommand(draft, { sceneId: props.sceneId, regionId: props.item.id, backgroundId: props.item.asset.id });
      if (!value) return;
      const { sceneId: _scene, regionId: _region, backgroundId: _background, ...action } = value;
      return action;
    },
    historyAvailable: redo => Boolean((redo ? region().drawingHistory?.redo : region().drawingHistory?.undo)?.length),
    historyStep: redo => (redo ? region().drawingHistory?.redo : region().drawingHistory?.undo)?.at(-1),
    history: () => region().drawingHistory ?? { undo: [], redo: [] },
    render: (drawing, interactive, selected) => <DrawingShape drawing={drawing} interactive={interactive} selected={selected} richContext={context()} onError={props.onError} />,
    copyTable: (drawing, format) => {
      const target = richPreviewTarget(context(), drawing); if (!target) throw Error('表格已更新');
      return props.onCopyTable(target, format);
    },
  };
  const command = (action: EditorDrawingAction) => {
    if (action.type === 'move_stored') return Promise.reject(Error('图片绘制操作无效'));
    const { expectedRevision, ...edit } = action;
    return props.onCommand({ type: 'edit_image_annotations', sceneId: props.sceneId, itemId: props.item.id, sourceAssetId: props.item.asset.id, expectedRevision, action: edit });
  };
  return <SharedDrawingEditor objectSource={()=>({sceneId:props.sceneId,itemId:props.item.id})} port={port} box={props.box} tools={coreDrawingTools.filter(kind => !['mosaic','highlighter','mark'].includes(kind))} busy={props.busy} inputLocked={props.inputLocked} onCommand={command} onError={props.onError} onClose={props.onClose} onExport={props.onExport} onPin={props.onPin} onRegisterFlush={props.onRegisterFlush} onRegisterHistory={props.onRegisterHistory} />;
}
