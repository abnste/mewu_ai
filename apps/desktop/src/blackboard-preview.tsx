// SPDX-License-Identifier: MPL-2.0
import { For } from 'solid-js';
import { render } from 'solid-js/web';
import type { Scene } from './contracts';
import { DrawingShape } from './components/DrawingLayer';

/** Browser-only preview. Desktop uses the checked native screenshot compositor. */
export async function blackboardPreview(scene: Scene): Promise<string> {
  const background=scene.background,region=scene.regions[0];
  if(!background?.width||!background.height||!region)throw Error('黑板不存在');
  if(region.drawings?.some(drawing=>drawing.kind==='rich'||drawing.kind==='mosaic'))throw Error('浏览器预览不支持合成此图层');
  const container=document.createElement('div');
  const dispose=render(()=><svg xmlns="http://www.w3.org/2000/svg" width={background.width} height={background.height} viewBox={`0 0 ${background.width} ${background.height}`}><For each={region.drawings??[]}>{drawing=><DrawingShape drawing={drawing}/>}</For></svg>,container);
  try{
    const xml=new XMLSerializer().serializeToString(container.firstElementChild!);
    const load=(path:string)=>new Promise<HTMLImageElement>((resolve,reject)=>{const image=new Image();image.onload=()=>resolve(image);image.onerror=()=>reject(Error('无法合成黑板'));image.src=path;});
    const [source,ink]=await Promise.all([load(background.path),load('data:image/svg+xml;charset=utf-8,'+encodeURIComponent(xml))]);
    const canvas=document.createElement('canvas');canvas.width=background.width;canvas.height=background.height;
    const context=canvas.getContext('2d');if(!context)throw Error('无法合成黑板');
    context.drawImage(source,0,0);context.drawImage(ink,0,0);
    return canvas.toDataURL('image/png');
  }finally{dispose();}
}
