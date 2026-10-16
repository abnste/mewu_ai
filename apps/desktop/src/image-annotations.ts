// SPDX-License-Identifier: MPL-2.0
import type { Region, SpaceItem } from './contracts';
export interface ImageContentBox { x:number;y:number;width:number;height:number }
/** The actual img uses object-fit:contain, centered inside its rendered shell. */
export function imageContentBox(slot:ImageContentBox, source:{width:number;height:number}):ImageContentBox {
  if(![slot.x,slot.y,slot.width,slot.height,source.width,source.height].every(Number.isFinite)||slot.width<=0||slot.height<=0||source.width<=0||source.height<=0)throw Error('图片显示尺寸无效');
  const scale=Math.min(slot.width/source.width,slot.height/source.height),width=source.width*scale,height=source.height*scale;
  return {x:slot.x+(slot.width-width)/2,y:slot.y+(slot.height-height)/2,width,height};
}
/** Preserve the existing raster shell/native pin contract: fit from top-left. */
export function imageObjectBox(item:SpaceItem,viewport:{width:number;height:number}):ImageContentBox {
  const ratio=(item.asset.width??1)/(item.asset.height??1),width=Math.min(item.width*viewport.width,item.height*viewport.height*ratio);
  return imageContentBox({x:item.x*viewport.width,y:item.y*viewport.height,width,height:width/ratio},{width:item.asset.width??1,height:item.asset.height??1});
}
export function imageAnnotationRegion(item: SpaceItem): Region {
  if (item.asset.kind !== 'image' || !item.asset.width || !item.asset.height) throw Error('图片来源无效');
  const document = item.imageAnnotations;
  if (document && (document.version !== 1 || document.sourceAssetId !== item.asset.id)) throw Error('图片标注来源已变化');
  return { id: item.id, x: 0, y: 0, width: item.asset.width, height: item.asset.height,
    drawings: document?.drawings ?? [], drawingRevision: document?.revision ?? 0, drawingHistory: document?.history ?? { undo: [], redo: [] } };
}
