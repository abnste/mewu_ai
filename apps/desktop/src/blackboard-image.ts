/* SPDX-License-Identifier: MPL-2.0 */
import type {Drawing,DrawingPoint} from './contracts';
import type {EditorBox} from './drawing-editor-port';
export const blackboardImage = (drawing:Drawing) => drawing.kind==='rich'&&drawing.rich?.kind==='extracted';
export function imageAt(drawings:Drawing[],point:DrawingPoint):Drawing|undefined{
  return [...drawings].reverse().find(drawing=>{if(!blackboardImage(drawing))return false;const [a,b]=drawing.points;return point.x>=a.x&&point.y>=a.y&&point.x<=b.x&&point.y<=b.y;});
}
export function zoomBlackboardImage(drawing:Drawing,frame:EditorBox,anchor:DrawingPoint,delta:number):Drawing|undefined{
  if(!blackboardImage(drawing)||!Number.isFinite(delta)||delta===0)return;
  const [a,b]=drawing.points,w=b.x-a.x,h=b.y-a.y;
  if(![a.x,a.y,b.x,b.y,frame.x,frame.y,frame.width,frame.height,anchor.x,anchor.y].every(Number.isFinite)||w<=0||h<=0||frame.width<=0||frame.height<=0)return;
  const max=Math.min(frame.width/w,frame.height/h),min=Math.min(max,24/Math.min(w,h));
  const factor=Math.max(min,Math.min(max,Math.exp(-Math.max(-240,Math.min(240,delta))*.002)));
  const width=w*factor,height=h*factor;
  const x=Math.max(frame.x,Math.min(frame.x+frame.width-width,anchor.x-(anchor.x-a.x)*factor));
  const y=Math.max(frame.y,Math.min(frame.y+frame.height-height,anchor.y-(anchor.y-a.y)*factor));
  return {...drawing,points:[{x,y},{x:x+width,y:y+height}]};
}
