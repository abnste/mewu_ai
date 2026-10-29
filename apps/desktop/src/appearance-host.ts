// SPDX-License-Identifier: MPL-2.0
import { createSignal } from 'solid-js';
import { invoke,isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AppearanceController,type AppearanceState } from './appearance-controller';
import { appearanceChanged,appearanceKey,appearanceProperties,readAppearanceSelection,type AppearanceChoice,type AppearanceRecord,type AppearanceSelection } from './appearance';
const [state,setState]=createSignal<AppearanceState>({choices:[],selection:null,active:null,ready:false});
export const appearanceState=state;
let controller:AppearanceController|undefined;
let applied='',owned:string[]=[];
const before=new Map<string,string>();
function apply(choice:AppearanceChoice|null){
  const identity=choice?JSON.stringify([choice.pluginId,choice.serviceId,choice.revision,choice.theme]):'';
  if(identity===applied)return;applied=identity;
  const root=document.documentElement;for(const css of owned){const value=before.get(css);value?root.style.setProperty(css,value):root.style.removeProperty(css);}owned=[];
  if(!choice){delete root.dataset.mewuTheme;delete root.dataset.mewuThemeBase;return;}
  const properties=appearanceProperties(choice.theme);for(const [css,value] of Object.entries(properties)){if(!before.has(css))before.set(css,root.style.getPropertyValue(css));root.style.setProperty(css,value);}owned=Object.keys(properties);
  root.dataset.mewuTheme=JSON.stringify([choice.pluginId,choice.serviceId]);root.dataset.mewuThemeBase=choice.theme.base;
}
function read(){try{return readAppearanceSelection(localStorage.getItem(appearanceKey));}catch{return null;}}
export async function initializeAppearance(){
  if(controller)return;
  controller=new AppearanceController({get:()=>isTauri()?invoke('get_extension_appearance'):Promise.resolve([]),subscribe:accept=>isTauri()?listen<AppearanceRecord[]>('extension-appearance',event=>accept(event.payload)):Promise.resolve(()=>{}),apply,publish:setState});
  controller.select(read());
  const sync=(event:StorageEvent|Event)=>{if(event instanceof StorageEvent&&event.key!==appearanceKey&&event.key!==null)return;controller?.select(read());};
  window.addEventListener('storage',sync);window.addEventListener(appearanceChanged,sync);
  window.addEventListener('pagehide',()=>{controller?.dispose();window.removeEventListener('storage',sync);window.removeEventListener(appearanceChanged,sync);},{once:true});
  await controller.initialize();
}
export function chooseAppearance(selection:AppearanceSelection|null){
  if(selection&&!state().choices.some(v=>v.pluginId===selection.pluginId&&v.serviceId===selection.serviceId))return;
  try{if(selection)localStorage.setItem(appearanceKey,JSON.stringify(selection));else localStorage.removeItem(appearanceKey);}catch{return;}
  window.dispatchEvent(new Event(appearanceChanged));
}
