// SPDX-License-Identifier: MPL-2.0
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {parse} from '@babel/parser';
import {stripTypeScriptTypes} from 'node:module';
import {runInNewContext} from 'node:vm';

test('finishing a selection keeps Enter with the screenshot; only explicit composer focus requests take focus',async()=>{
  const source=await readFile(new URL('./App.tsx',import.meta.url),'utf8');
  const nodes=[];
  function walk(node){if(!node||typeof node!=='object')return;if(node.type==='FunctionDeclaration')nodes.push(node);for(const [key,value] of Object.entries(node)){if(key==='loc')continue;if(Array.isArray(value))value.forEach(walk);else if(value&&typeof value==='object')walk(value);}}
  walk(parse(source,{sourceType:'module',plugins:['typescript','jsx']}));
  const handler=nodes.find(node=>node.id?.name==='selectionCompleted');assert.ok(handler);
  const completed=[],focus=[];let recording=false;
  const select=runInNewContext(stripTypeScriptTypes(source.slice(handler.start,handler.end),{mode:'transform'})+';selectionCompleted',{
    recordingSelection:{completed:id=>{completed.push(id);return recording;}},setFocusRequest:value=>focus.push(value),
  });
  select({id:'capture'});recording=true;select({id:'record'});
  assert.deepEqual(completed,['capture','record']);assert.deepEqual(focus,[]);
});

test('finishing region or image drawing returns keyboard ownership to the canvas, not the composer', async () => {
  const source = await readFile(new URL('./components/SpaceCanvas.tsx', import.meta.url), 'utf8');
  const nodes = [];
  function walk(node) {
    if (!node || typeof node !== 'object') return;
    nodes.push(node);
    for (const [key, value] of Object.entries(node)) {
      if (key === 'loc') continue;
      if (Array.isArray(value)) value.forEach(walk);
      else if (value && typeof value === 'object') walk(value);
    }
  }
  walk(parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] }));
  const region = { id: 'selected' }, calls = [];
  const props = {
    blackboard: false,
    onFocusComposer: () => calls.push('composer'),
    onBlackboardClose: () => calls.push('blackboard'),
  };
  const bindings = {
    props,
    setDrawingSession: value => calls.push(['region', value]),
    setImageDrawing: value => calls.push(['image', value]),
    setSelectionActivity: value => calls.push(['activity', value]),
    activeRegion: () => region,
    revealToolbar: value => calls.push(['toolbar', value.id]),
    canvas: { focus: options => calls.push(['canvas', options.preventScroll]) },
  };
  const closeNode = nodes.find(node => node.type === 'FunctionDeclaration' && node.id?.name === 'closeDrawing');
  const imageNode = nodes.find(node => node.type === 'VariableDeclarator' && node.id?.name === 'closeImageDrawing');
  assert.ok(closeNode); assert.ok(imageNode);
  const close = runInNewContext(stripTypeScriptTypes(source.slice(closeNode.start, closeNode.end), { mode: 'transform' }) + ';closeDrawing', bindings);
  const closeImage = runInNewContext('(' + source.slice(imageNode.init.start, imageNode.init.end) + ')', bindings);
  close();
  assert.deepEqual(calls, [['region', undefined], ['activity', false], ['toolbar', region.id], ['canvas', true]]);
  calls.length = 0;
  closeImage();
  assert.deepEqual(calls, [['image', undefined], ['activity', false], ['canvas', true]]);
  calls.length = 0;
  props.blackboard = true;
  close();
  assert.deepEqual(calls, ['blackboard'], 'leaving the blackboard keeps its existing owner handoff');

  const referenceButton = nodes.find(node => node.type === 'JSXOpeningElement' && node.attributes?.some(attribute => attribute.name?.name === 'data-layout-id' && attribute.value?.value === 'reference'));
  const referenceAction = referenceButton?.attributes.find(attribute => attribute.name?.name === 'onClick')?.value.expression;
  assert.ok(referenceAction);
  const references = [];
  const reference = runInNewContext('(' + source.slice(referenceAction.start, referenceAction.end) + ')', {
    region: () => region, referenced: () => false,
    props: { onReference: value => references.push(value.id), onFocusComposer: () => calls.push('composer') },
  });
  calls.length = 0;
  reference();
  assert.deepEqual(references, [region.id]);
  assert.deepEqual(calls, ['composer'], 'only the explicit reference action requests composer focus');
});
