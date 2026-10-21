// SPDX-License-Identifier: MPL-2.0
export interface DocumentBounds {x:number;y:number;width:number;height:number;viewportWidth?:boolean;viewportHeight?:boolean}
export function documentBounds(value:unknown):DocumentBounds|undefined {
  if(!value||typeof value!=='object')return;
  const v=value as DocumentBounds;
  if(![v.x,v.y,v.width,v.height].every(Number.isFinite)||Math.abs(v.x)>32768||Math.abs(v.y)>32768||v.width<1||v.height<1||v.width>32768||v.height>32768)return;
  return {x:v.x,y:v.y,width:v.width,height:v.height,...(v.viewportWidth===true?{viewportWidth:true}:{}),...(v.viewportHeight===true?{viewportHeight:true}:{})};
}
export function documentDisplay(box:DocumentBounds) {
  // The authored document decides its size. Never shrink full-screen content
  // or reinterpret its viewport units as a fraction of an arbitrary card.
  return {width:box.width,height:box.height,scale:1};
}
