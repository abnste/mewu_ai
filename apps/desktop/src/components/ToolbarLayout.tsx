// SPDX-License-Identifier: MPL-2.0
import { children, createEffect, createMemo, createSignal, For, onCleanup, type JSX } from 'solid-js';
import * as api from '../extension-bridge';
import { resolveLanguage, uiLanguage } from '../i18n';
import { reorderToolbar } from '../toolbar-layout';
import { toolbarDefinition, type ToolbarItem } from '../toolbar-catalog';
import { cachedToolbarLayout, resolveToolbarLayout, startToolbarLayouts, subscribeToolbarLayouts, toolbarLayoutOwner, toolbarLayoutsReady } from '../toolbar-layout-session';

/** Move existing nodes through Solid's keyed list, never rebuild their handlers
 * or use CSS order (which would leave keyboard navigation in the wrong order). */
export default function ToolbarLayout(props: { surface: string; children: JSX.Element; onReady?:(ready:boolean)=>void }) {
  const resolved = children(() => props.children);
  const nodes = createMemo(() => resolved.toArray());
  const [refresh, setRefresh] = createSignal(0), [ready,setReady]=createSignal(false);
  const [captions,setCaptions]=createSignal(0);
  const [plans, setPlans] = createSignal<unknown[]>([]);
  let disposed = false, generation = 0;
  const id = (node: JSX.Element) => node instanceof HTMLElement ? node.dataset.layoutId : undefined;
  const items = createMemo(() => {
    captions();
    const definitions=toolbarDefinition(props.surface);
    return nodes().flatMap(node=>{const key=id(node);if(!key)return[];
      const definition=definitions.find(item=>item.id===key),element=node as HTMLElement;
      const declared=element.dataset.layoutKind;
      return [{id:key,kind:declared==='separator'||declared==='button'?declared:definition?.kind??'button',title:element.getAttribute('data-caption')||element.getAttribute('aria-label')||definition?.title||key} as ToolbarItem];
    });
  }, undefined, { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) });
  const ordered = createMemo(() => ready()?plans().reduce<JSX.Element[]>((value, result) => reorderToolbar(value, id, result), nodes()):[]);
  let owner = '';
  const stop=subscribeToolbarLayouts(()=>setRefresh(value=>value+1)),release=startToolbarLayouts();
  createEffect(()=>props.onReady?.(ready()));
  createEffect(() => {
    const source = items(), surface = props.surface;
    refresh(); const current = ++generation;
    if(!toolbarLayoutsReady())return;
    const identity=toolbarLayoutOwner(surface);
    if(owner!==identity){owner=identity;setPlans([]);setReady(false);}
    const cached=cachedToolbarLayout(surface,source);
    if(cached){setPlans(cached);setReady(true);return;}
    void resolveToolbarLayout(surface,source).then(value=>{
      if(!disposed&&current===generation){setPlans(value);setReady(true);}
    },()=>{/* A changed owner triggers a new request through the subscription. */});
  });
  createEffect(()=>{const source=items(),locale=resolveLanguage(uiLanguage());if(api.extensionNative)void api.publishToolbarCatalog(props.surface,locale,source).catch(()=>{});});
  createEffect(()=>{
    const current=nodes();if(typeof MutationObserver==='undefined')return;
    const observer=new MutationObserver(()=>setCaptions(value=>value+1));
    for(const node of current)if(node instanceof HTMLElement)observer.observe(node,{attributes:true,attributeFilter:['data-caption','aria-label','data-layout-kind']});
    onCleanup(()=>observer.disconnect());
  });
  onCleanup(() => { disposed = true; generation++; stop();release(); });
  return <For each={ordered()}>{node => node}</For>;
}
