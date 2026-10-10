// SPDX-License-Identifier: MPL-2.0
export const OBJECT_MOVE_WIDTH = 56;
export const OBJECT_MOVE_HEIGHT = 24;
export function objectMoveGripPosition(box: {x:number;y:number;width:number}, viewport: {width:number;height:number}) {
  const center=Math.max(OBJECT_MOVE_WIDTH/2,Math.min(viewport.width-OBJECT_MOVE_WIDTH/2,box.x+box.width/2));
  const top=Math.max(4,Math.min(viewport.height-OBJECT_MOVE_HEIGHT,box.y+4));
  return {left:`${center-box.x}px`,top:`${top-box.y}px`};
}
