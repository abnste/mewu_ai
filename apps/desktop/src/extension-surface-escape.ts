// SPDX-License-Identifier: MPL-2.0
// Shared by all plugin panels. Native object windows own their own Escape.
const panels=new Map<object,()=>void>();
export function registerSurfaceEscape(close:()=>void){
  const key={};panels.set(key,close);
  return {focus(){panels.delete(key);panels.set(key,close);},dispose(){panels.delete(key);}};
}
export function closeTopSurface(){
  const close=[...panels.values()].at(-1);if(!close)return false;close();return true;
}
