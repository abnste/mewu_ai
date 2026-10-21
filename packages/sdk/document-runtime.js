// SPDX-License-Identifier: MPL-2.0
// Runs only inside the opaque, network-disabled document frame. It grants no host API.
function mewuDocumentRuntime() {
  let port, scheduled = false, last = '', disposed = false, offsetX=0, offsetY=0;
  // Preserve the authored layout, including viewport units and full-screen
  // content. Only remove invisible offsets from the object's drag boundary.
  const hidden = new Set(['SCRIPT','STYLE','LINK','META','HEAD','TITLE','NOSCRIPT']);
  const style = document.createElement('style');
  style.textContent = ':where(html,body){margin:0;background:transparent}:where(body){font-family:system-ui,sans-serif;color:#f1f5f9}html,body{overflow:hidden!important}body>svg{overflow:visible}';
  document.head.prepend(style);
  function bounds() {
    let left=Infinity,top=Infinity,right=-Infinity,bottom=-Infinity,count=0;
    function include(r){if(r.width>0&&r.height>0&&[r.left,r.top,r.right,r.bottom].every(Number.isFinite)){left=Math.min(left,r.left+offsetX);top=Math.min(top,r.top+offsetY);right=Math.max(right,r.right+offsetX);bottom=Math.max(bottom,r.bottom+offsetY);}}
    const authored=document.querySelector('[data-mewu-object]');
    if(authored){const r=authored.getBoundingClientRect();if(r.width>0&&r.height>0)return{x:r.left+offsetX,y:r.top+offsetY,width:r.width,height:r.height,viewportWidth:Math.abs(r.width-innerWidth)<2,viewportHeight:Math.abs(r.height-innerHeight)<2};}
    for(const node of [document.body,...document.body.querySelectorAll('*')]) {
      if(++count>4000)break;
      if(hidden.has(node.tagName)||node.closest('svg')!==null&&node.tagName.toLowerCase()!=='svg')continue;
      const s=getComputedStyle(node);if(s.display==='none'||s.visibility==='hidden'||Number(s.opacity)===0)continue;
      const replaced=['SVG','CANVAS','IMG','VIDEO','INPUT','BUTTON','SELECT','TEXTAREA'].includes(node.tagName.toUpperCase());
      const painted=s.backgroundColor!=='rgba(0, 0, 0, 0)'&&s.backgroundColor!=='transparent'||s.backgroundImage!=='none'||parseFloat(s.borderTopWidth)>0;
      if(node instanceof SVGSVGElement&&!painted){
        try{const b=node.getBBox(),m=node.getScreenCTM();if(m&&b.width>0&&b.height>0){const points=[[b.x,b.y],[b.x+b.width,b.y],[b.x,b.y+b.height],[b.x+b.width,b.y+b.height]].map(([x,y])=>new DOMPoint(x,y).matrixTransform(m));const xs=points.map(p=>p.x),ys=points.map(p=>p.y);include({left:Math.min(...xs),top:Math.min(...ys),right:Math.max(...xs),bottom:Math.max(...ys),width:Math.max(...xs)-Math.min(...xs),height:Math.max(...ys)-Math.min(...ys)});}else include(node.getBoundingClientRect());}catch{include(node.getBoundingClientRect());}
      }else if(replaced||painted)include(node.getBoundingClientRect());
      for(const child of node.childNodes)if(child.nodeType===Node.TEXT_NODE&&child.textContent.trim()){const range=document.createRange();range.selectNodeContents(child);for(const rect of range.getClientRects())include(rect);}
    }
    for(const child of document.body.childNodes)if(child.nodeType===Node.TEXT_NODE&&child.textContent.trim()){const r=document.createRange();r.selectNodeContents(child);include(r.getBoundingClientRect());}
    if(!Number.isFinite(left))return {x:0,y:0,width:innerWidth,height:innerHeight};
    const width=Math.ceil(right-left),height=Math.ceil(bottom-top);
    return {x:Math.floor(left),y:Math.floor(top),width,height,viewportWidth:Math.abs(width-innerWidth)<2,viewportHeight:Math.abs(height-innerHeight)<2};
  }
  function contrast() {
    const rgb=v=>{const m=v.match(/^rgba?\(([^)]+)\)$/);if(!m)return;const a=m[1].split(/[, /]+/).map(Number);return a.length<4||a[3]>=.95?a:undefined;};
    const light=c=>{const a=c.slice(0,3).map(n=>{n/=255;return n<=.04045?n/12.92:((n+.055)/1.055)**2.4;});return a[0]*.2126+a[1]*.7152+a[2]*.0722;};
    let count=0;
    for(const node of [document.body,...document.body.querySelectorAll('*')]){
      if(++count>4000)break;if(hidden.has(node.tagName)||!node.textContent.trim()||![...node.childNodes].some(n=>n.nodeType===Node.TEXT_NODE&&n.textContent.trim()))continue;
      let parent=node,bg;while(parent&&!bg){bg=rgb(getComputedStyle(parent).backgroundColor);parent=parent.parentElement;}bg??=[48,50,54];
      const s=getComputedStyle(node),fg=rgb(node instanceof SVGElement?s.fill:s.color);if(!fg)continue;
      if(node instanceof SVGElement){const box=node.getBoundingClientRect();for(const rect of node.ownerSVGElement?.querySelectorAll('rect')??[]){const r=rect.getBoundingClientRect(),fill=rgb(getComputedStyle(rect).fill);if(fill&&r.left<=box.left&&r.right>=box.right&&r.top<=box.top&&r.bottom>=box.bottom)bg=fill;}}
      const a=light(fg),b=light(bg);
      if((Math.max(a,b)+.05)/(Math.min(a,b)+.05)<3){const color=b>.42?'#17202b':'#f1f5f9';node.style.color=color;if(node instanceof SVGElement)node.style.fill=color;}
    }
  }
  function expandScrollContainers(){
    if(document.querySelector('[data-mewu-object]'))return;
    let count=0;
    for(const node of document.body.querySelectorAll('*')){
      if(++count>4000)break;
      if(!(node instanceof HTMLElement)||['TEXTAREA','INPUT','SELECT'].includes(node.tagName))continue;
      const css=getComputedStyle(node);
      if(/auto|scroll/.test(css.overflowY)&&node.scrollHeight>node.clientHeight){node.style.setProperty('overflow-y','visible','important');node.style.setProperty('max-height','none','important');node.style.setProperty('height','auto','important');}
      if(/auto|scroll/.test(css.overflowX)&&node.scrollWidth>node.clientWidth){node.style.setProperty('overflow-x','visible','important');node.style.setProperty('max-width','none','important');node.style.setProperty('width',Math.min(32768,node.scrollWidth)+'px','important');}
    }
  }
  function report(){scheduled=false;if(disposed||!port)return;expandScrollContainers();contrast();const box=bounds(),key=JSON.stringify(box);if(key!==last){last=key;offsetX=box.x;offsetY=box.y;document.body.style.translate=(-offsetX)+'px '+(-offsetY)+'px';port.postMessage({type:'layout',...box});}}
  function schedule(){if(!scheduled&&!disposed){scheduled=true;requestAnimationFrame(report);}}
  async function preview(){
    const box=bounds(),clone=document.body.cloneNode(true),original=[document.body,...document.body.querySelectorAll('*')],copies=[clone,...clone.querySelectorAll('*')];
    if(original.length>4000)throw Error('Document too large');
    for(let i=0;i<original.length;i++){
      const from=original[i],to=copies[i];if(hidden.has(from.tagName)){to.remove();continue;}
      for(const name of to.getAttributeNames())if(/^on/i.test(name)||['srcdoc','href','src'].includes(name))to.removeAttribute(name);
      const css=getComputedStyle(from);to.setAttribute('style',[...css].map(k=>k+':'+css.getPropertyValue(k)).join(';'));
      if(from instanceof HTMLCanvasElement){const img=document.createElement('img');img.src=from.toDataURL('image/png');img.style.cssText=to.style.cssText;to.replaceWith(img);}
      else if(from instanceof HTMLImageElement&&/^data:image\/(png|jpeg|webp);base64,/.test(from.src))to.setAttribute('src',from.src);
      else if(from instanceof HTMLInputElement)to.setAttribute('value',from.value);
      else if(from instanceof HTMLTextAreaElement)to.textContent=from.value;
    }
    clone.style.cssText+=';margin:0!important;position:relative!important;transform:none!important;translate:none!important;overflow:visible!important';
    const xml=new XMLSerializer().serializeToString(clone);
    if(xml.length>1024*1024)throw Error('Preview too large');
    const stageWidth=Math.max(innerWidth,document.body.scrollWidth,box.x+box.width),stageHeight=Math.max(innerHeight,document.body.scrollHeight,box.y+box.height);
    const svg='<svg xmlns="http://www.w3.org/2000/svg" width="240" height="180" viewBox="'+[box.x,box.y,box.width,box.height].join(' ')+'"><foreignObject x="0" y="0" width="'+stageWidth+'" height="'+stageHeight+'">'+xml+'</foreignObject></svg>';
    const image='data:image/svg+xml;base64,'+btoa(unescape(encodeURIComponent(svg)));
    port?.postMessage({type:'preview',image});
  }
  addEventListener('message',event=>{if(event.source!==parent||event.data?.type!=='mewu-document-connect'||!event.ports[0]||port)return;port=event.ports[0];port.onmessage=event=>{if(event.data?.type==='preview')void preview().catch(()=>port?.postMessage({type:'preview',image:null}));};port.start();report();});
  const resize=new ResizeObserver(schedule),mutation=new MutationObserver(schedule);
  resize.observe(document.body);mutation.observe(document.body,{subtree:true,childList:true,characterData:true,attributes:true,attributeFilter:['style','class','width','height','hidden']});
  const authored=document.querySelector('[data-mewu-object]');if(authored)resize.observe(authored);
  addEventListener('resize',schedule);addEventListener('load',schedule,true);document.fonts?.ready.then(schedule);
  addEventListener('pagehide',()=>{disposed=true;resize.disconnect();mutation.disconnect();port?.close();},{once:true});
}
