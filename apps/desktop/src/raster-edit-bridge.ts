// SPDX-License-Identifier: MPL-2.0
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Asset, DrawingPoint, Region, Snapshot } from './contracts';

export interface RasterEditTarget { sceneId:string;background:Asset;region:Region }
export interface RasterEditReceipt { snapshot:Snapshot;selectedId?:string|null }
export async function applyRasterEdit(target:RasterEditTarget,mode:'extract'|'heal',points:DrawingPoint[],width:number):Promise<RasterEditReceipt> {
  if (!isTauri()) throw Error('此操作需要桌面应用');
  // Freeze the complete target bytes before IPC; later props never reinterpret a stroke.
  const copy=structuredClone(target), stroke=points.map(p=>({...p}));
  if ((mode==='extract' && stroke.length!==2) || !stroke.length || stroke.length>4096 || !Number.isFinite(width)
    || stroke.some(p=>!Number.isFinite(p.x)||!Number.isFinite(p.y))) throw Error('修补笔划无效');
  return invoke<RasterEditReceipt>('apply_raster_edit',{target:copy,mode,points:stroke,width});
}
