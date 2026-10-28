// SPDX-License-Identifier: MPL-2.0
import type { SnapBounds,SnapNode } from './window-snap';
/** Advisory image targets only. They never carry a native window witness or
 * grant input/scroll authority; the original crop/copy path validates geometry. */
export function quickImageTarget(images:readonly SnapBounds[]|undefined,width:number,height:number,x:number,y:number):SnapNode|undefined{
  if(!images||images.length>128||!Number.isFinite(x)||!Number.isFinite(y))return;
  let selected:SnapNode|undefined,area=Infinity;
  images.forEach((bounds,index)=>{
    if(![bounds.x,bounds.y,bounds.width,bounds.height].every(Number.isSafeInteger)||bounds.x<0||bounds.y<0||bounds.width<1||bounds.height<1||bounds.x+bounds.width>width||bounds.y+bounds.height>height)return;
    const size=bounds.width*bounds.height;
    if(x>=bounds.x&&y>=bounds.y&&x<bounds.x+bounds.width&&y<bounds.y+bounds.height&&size<area){area=size;selected={id:0x80000000+index,bounds:{...bounds},selectable:true,snapshot:false,children:[]};}
  });return selected;
}
