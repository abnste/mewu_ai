/* SPDX-License-Identifier: MPL-2.0 */
export const colorPresets = ['#FFFFFF','#242832','#EE4848','#F28B38','#F4C94D','#39A97C','#348BF1','#9467D9'];
export const colorHistoryKey = 'mewu.drawing-colors.v1';
export const normalizeColor = (value:string) => /^#[\da-f]{6}$/i.test(value.trim()) ? value.trim().toUpperCase() : undefined;
export const colorRgb = (value:string) => [1,3,5].map(offset=>parseInt(value.slice(offset,offset+2),16));
export function rgbColor(channels:number[]) { return '#'+channels.map(value=>Math.round(Math.max(0,Math.min(255,value))).toString(16).padStart(2,'0')).join('').toUpperCase(); }
export function colorHsv(value:string) {
  const [r,g,b]=colorRgb(value).map(value=>value/255), max=Math.max(r,g,b),min=Math.min(r,g,b),delta=max-min;
  const hue=delta===0?0:max===r?((g-b)/delta)%6:max===g?(b-r)/delta+2:(r-g)/delta+4;
  return {h:(hue*60+360)%360,s:max===0?0:delta/max,v:max};
}
export function hsvColor(h:number,s:number,v:number) {
  const hue=((h%360)+360)%360/60, c=v*s, x=c*(1-Math.abs(hue%2-1)),m=v-c;
  const rgb=hue<1?[c,x,0]:hue<2?[x,c,0]:hue<3?[0,c,x]:hue<4?[0,x,c]:hue<5?[x,0,c]:[c,0,x];
  return rgbColor(rgb.map(value=>(value+m)*255));
}
export function recentColors(storage:Pick<Storage,'getItem'>=localStorage):string[] {
  try { const values:unknown=JSON.parse(storage.getItem(colorHistoryKey)||'[]'); return Array.isArray(values)?[...new Set(values.filter((value):value is string=>typeof value==='string').map(normalizeColor).filter((value):value is string=>!!value))].slice(0,3):[]; } catch { return []; }
}
export function rememberColor(value:string,storage:Pick<Storage,'getItem'|'setItem'>=localStorage):string[] {
  const normalized=normalizeColor(value),before=recentColors(storage);if(!normalized)return before;
  const next=[normalized,...before.filter(value=>value!==normalized)].slice(0,3);
  try{storage.setItem(colorHistoryKey,JSON.stringify(next));}catch{/* Drawing remains usable when local storage is unavailable. */}
  return next;
}
