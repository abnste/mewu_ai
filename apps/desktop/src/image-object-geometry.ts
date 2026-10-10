// SPDX-License-Identifier: MPL-2.0
import type {EditorBox} from './drawing-editor-port';
/** Viewport fractions; keep a small reachable edge on every side. */
export function moveImageObject(box:EditorBox,dx:number,dy:number,vw:number,vh:number):EditorBox {
  const edgeX=Math.min(16/vw,box.width),edgeY=Math.min(16/vh,box.height);
  return {...box,x:Math.max(edgeX-box.width,Math.min(1-edgeX,box.x+dx)),y:Math.max(edgeY-box.height,Math.min(1-edgeY,box.y+dy))};
}
export function zoomImageObject(box:EditorBox,anchor:{x:number;y:number},delta:number,vw:number,vh:number):EditorBox|undefined {
  if(![box.x,box.y,box.width,box.height,anchor.x,anchor.y,delta,vw,vh].every(Number.isFinite)||box.width<=0||box.height<=0||vw<=0||vh<=0||delta===0)return;
  const shortest=Math.min(box.width*vw,box.height*vh),longest=Math.max(box.width,box.height);
  const factor=Math.max(Math.min(1,24/shortest),Math.min(8/longest,Math.exp(-Math.max(-240,Math.min(240,delta))*.002)));
  const next={x:anchor.x-(anchor.x-box.x)*factor,y:anchor.y-(anchor.y-box.y)*factor,width:box.width*factor,height:box.height*factor};
  return moveImageObject(next,0,0,vw,vh);
}
