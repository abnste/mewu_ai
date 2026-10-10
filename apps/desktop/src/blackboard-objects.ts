// SPDX-License-Identifier: MPL-2.0
import type {Scene,SpaceItem} from './contracts';
/** Browser preview follows the host's independent object copies; assets remain immutable. */
export function carryBlackboardObjects(scene:Scene,width:number,height:number,id:()=>string):SpaceItem[] {
  return scene.items.map(original=>{
    const item=structuredClone(original);item.id=id();item.state??={};
    const source=scene.background;
    if(source&&item.state.coordinateSpace==='background'&&item.state.backgroundId===source.id){
      if(source.width&&source.height){const scale=Math.min(width/source.width,height/source.height);item.width=Math.min(1,item.width*source.width*scale/width);item.height=Math.min(1,item.height*source.height*scale/height);item.x=Math.max(0,Math.min(1-item.width,((width-source.width*scale)/2+item.x*source.width*scale)/width));item.y=Math.max(0,Math.min(1-item.height,((height-source.height*scale)/2+item.y*source.height*scale)/height));}
      delete item.state.coordinateSpace;delete item.state.backgroundId;
    }
    delete item.state.blackboardSceneId;
    item.state.__mewuBoardSource={sceneId:scene.id,itemId:original.id};return item;
  });
}
