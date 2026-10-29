// SPDX-License-Identifier: MPL-2.0
import contract from '../../../packages/sdk/appearance.json';
import type { AppearanceTheme } from '../../../packages/sdk/index';

export type { AppearanceTheme };
export interface AppearanceChoice { pluginId:string; serviceId:string; title:string; theme:AppearanceTheme; revision:number }
export interface AppearanceRecord { pluginId:string; revision:number; themes:Array<Omit<AppearanceChoice,'pluginId'|'revision'>> }
export interface AppearanceSelection { pluginId:string; serviceId:string }
export const appearanceKey='mewu.appearance.v1';
export const appearanceChanged='mewu:appearance';
const object=(v:unknown):v is Record<string,unknown>=>v!==null&&typeof v==='object'&&!Array.isArray(v);
const closed=(v:Record<string,unknown>,keys:string[])=>Object.keys(v).every(k=>keys.includes(k));
const id=(v:unknown):v is string=>typeof v==='string'&&v.length>0&&v.length<=128&&/^[a-zA-Z0-9_.-]+$/.test(v);
export const appearanceChoiceKey=(v:AppearanceSelection)=>JSON.stringify([v.pluginId,v.serviceId]);
export function readAppearanceSelection(value:string|null):AppearanceSelection|null {
  try { const v:unknown=JSON.parse(value??'null');return object(v)&&closed(v,['pluginId','serviceId'])&&id(v.pluginId)&&id(v.serviceId)?{pluginId:v.pluginId,serviceId:v.serviceId}:null; }catch{return null;}
}
export function parseAppearanceTheme(value:unknown):AppearanceTheme|null {
  if(!object(value)||!closed(value,['version','base','colors','backgrounds'])||value.version!==1||!['light','dark'].includes(String(value.base)))return null;
  const colors=value.colors??{},backgrounds=value.backgrounds??{};
  if(!object(colors)||!object(backgrounds)||!closed(colors,Object.keys(contract.colors))||!closed(backgrounds,contract.backgroundSurfaces))return null;
  if(Object.values(colors).some(v=>typeof v!=='string'||!/^#[0-9a-f]{6}$/i.test(v)))return null;
  for(const background of Object.values(backgrounds)){
    if(!object(background)||!closed(background,['png','fit'])||!['cover','contain','repeat'].includes(String(background.fit))||typeof background.png!=='string'||background.png.length<44||background.png.length>256*1024||background.png.length%4!==0)return null;
    try{
      const png=atob(background.png);if(btoa(png)!==background.png||png.length<33||[137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82].some((v,i)=>png.charCodeAt(i)!==v))return null;
      const number=(offset:number)=>((png.charCodeAt(offset)*0x1000000)+(png.charCodeAt(offset+1)<<16)+(png.charCodeAt(offset+2)<<8)+png.charCodeAt(offset+3));
      const w=number(16),h=number(20);if(w<1||h<1||w>4096||h>4096||w*h>8_000_000)return null;
    }catch{return null;}
  }
  return structuredClone(value) as unknown as AppearanceTheme;
}
/** Full records include empty tombstones. A delayed enabled event cannot
 * resurrect a contribution after a newer disable, removal or update. */
export function mergeAppearanceRecords(previous:AppearanceRecord[],incoming:AppearanceRecord[]):AppearanceRecord[]{
  const records=new Map(previous.map(r=>[r.pluginId,r]));
  for(const candidate of incoming){
    if(!object(candidate)||!id(candidate.pluginId)||!Number.isSafeInteger(candidate.revision)||candidate.revision<1||!Array.isArray(candidate.themes))continue;
    const old=records.get(candidate.pluginId);if(old&&old.revision>candidate.revision)continue;
    const seen=new Set<string>();
    const themes=candidate.themes.flatMap(value=>{
      if(!object(value)||!id(value.serviceId)||seen.has(value.serviceId)||typeof value.title!=='string'||value.title.length>128)return [];
      const theme=parseAppearanceTheme(value.theme);if(!theme)return [];seen.add(value.serviceId);return [{serviceId:value.serviceId,title:value.title,theme}];
    });
    records.set(candidate.pluginId,{pluginId:candidate.pluginId,revision:candidate.revision,themes});
  }
  return [...records.values()];
}
export function appearanceChoices(records:AppearanceRecord[]):AppearanceChoice[]{
  return records.flatMap(record=>record.themes.map(theme=>({...theme,pluginId:record.pluginId,revision:record.revision}))).sort((a,b)=>a.title.localeCompare(b.title)||appearanceChoiceKey(a).localeCompare(appearanceChoiceKey(b)));
}
/** Mutates only owned presentation properties. Native transparency, geometry,
 * captured pixels, drawings, plugin frames and user documents are untouched. */
export function appearanceProperties(theme:AppearanceTheme|null):Record<string,string>{
  if(!theme)return {};
  const result:Record<string,string>={};
  for(const [key,definition] of Object.entries(contract.colors))for(const css of definition.css) result[css]=theme.colors?.[key as keyof NonNullable<AppearanceTheme['colors']>]??definition[theme.base];
  for(const surface of contract.backgroundSurfaces){
    const value=theme.backgrounds?.[surface as keyof NonNullable<AppearanceTheme['backgrounds']>];
    result[`--theme-${surface}-image`]=value?`url("data:image/png;base64,${value.png}")`:'none';
    result[`--theme-${surface}-fit`]=value?.fit==='repeat'?'auto':value?.fit??'cover';
    result[`--theme-${surface}-repeat`]=value?.fit==='repeat'?'repeat':'no-repeat';
  }
  return result;
}
