// SPDX-License-Identifier: MPL-2.0
// Actual source geometry/layout contract, synthetic objects only. No native/model.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule } from 'node:vm';
import { parse } from '@babel/parser';
async function load(file) { const value=new SourceTextModule(stripTypeScriptTypes(await readFile(new URL(file,import.meta.url),'utf8'),{mode:'transform'}));await value.link(()=>{throw Error('Unexpected runtime dependency');});await value.evaluate();return value.namespace; }
const images=await load('./image-annotations.ts'),rich=await load('./drawing-layout-preview.ts');
const plain=value=>JSON.parse(JSON.stringify(value));
const id='11111111-1111-4111-8111-111111111111',source='22222222-2222-4222-8222-222222222222',scene='33333333-3333-4333-8333-333333333333';
const item=()=>({id,asset:{id:source,kind:'image',width:800,height:600,path:'synthetic.png'},x:-.1,y:.2,width:.7,height:.4});
test('contain geometry shares centered content offsets for non-square slots, zoom and negative positions',()=>{
  assert.deepEqual(plain(images.imageContentBox({x:10,y:20,width:400,height:400},{width:800,height:400})),{x:10,y:120,width:400,height:200});
  assert.deepEqual(plain(images.imageContentBox({x:-100,y:-30,width:600,height:300},{width:400,height:800})),{x:125,y:-30,width:150,height:300});
  const a=images.imageObjectBox(item(),{width:1200,height:800});assert.ok(Math.abs(a.x+120)<1e-10);assert.equal(a.y,160);assert.equal(a.height,320);assert.ok(Math.abs(a.width-320*4/3)<1e-10);
  const zoom=images.imageObjectBox({...item(),width:item().width*1.5,height:item().height*1.5},{width:1200,height:800});assert.ok(Math.abs(zoom.width/a.width-1.5)<1e-12);assert.ok(Math.abs(zoom.height/a.height-1.5)<1e-12);
  assert.throws(()=>images.imageContentBox({x:0,y:0,width:0,height:1},{width:1,height:1}));
});
test('image document coordinates stay in source pixels, independent from scene background or display bounds',()=>{
  const value=item();value.imageAnnotations={version:1,sourceAssetId:source,revision:7,drawings:[{id:'ink',kind:'line'}],history:{undo:[],redo:[]}};
  const original=JSON.stringify(value),region=images.imageAnnotationRegion(value);
  assert.deepEqual(plain(region),{id,x:0,y:0,width:800,height:600,drawings:value.imageAnnotations.drawings,drawingRevision:7,drawingHistory:{undo:[],redo:[]}});assert.equal(JSON.stringify(value),original);
  assert.throws(()=>images.imageAnnotationRegion({...value,imageAnnotations:{...value.imageAnnotations,sourceAssetId:scene}}));
  assert.throws(()=>images.imageAnnotationRegion({...value,asset:{...value.asset,kind:'video'}}));
});
test('rich image preview binds explicit item and source identities and cannot collide with a region request',()=>{
  const reference={layoutId:'44444444-4444-4444-8444-444444444444',layoutSha256:'a'.repeat(64),rasterSha256:'b'.repeat(64),kind:'formula',width:200,height:40};
  const value=item(),region=images.imageAnnotationRegion(value),drawing={id:'55555555-5555-4555-8555-555555555555',kind:'rich',rich:reference};
  region.drawings=[drawing];
  const image=rich.richPreviewTarget({sceneId:scene,background:value.asset,region,imageItemId:id},drawing),capture=rich.richPreviewTarget({sceneId:scene,background:value.asset,region},drawing);
  assert.equal(rich.validLayoutTarget(image),true);assert.equal(rich.validLayoutTarget(capture),true);assert.notEqual(rich.layoutRequestKey(image,''),rich.layoutRequestKey(capture,''));
  assert.equal(rich.validLayoutTarget({...image,imageItemId:scene}),false);assert.equal(rich.validLayoutTarget({...image,sourceAssetId:undefined}),false);assert.equal(rich.validLayoutTarget({...capture,sourceAssetId:source}),false);
});
test('image passive ink resets screenshot border compensation and does not intercept image interaction',async()=>{
  const css=await readFile(new URL('./components/image-object.css',import.meta.url),'utf8');
  assert.match(css,/\.image-annotations-layer\{[^}]*pointer-events:none/);assert.match(css,/\.image-annotations-layer>\.drawing-layer\{inset:0;width:100%;height:100%;pointer-events:none\}/);
  const editor=await readFile(new URL('./components/ImageDrawingEditor.tsx',import.meta.url),'utf8');assert.match(editor,/SharedDrawingEditor/);assert.match(editor,/!\['mosaic','highlighter','mark'\]\.includes\(kind\)/);
});
test('actual image editor sends one source-bound batch annotation edit and never removes the media item',async()=>{
  const sourceCode=await readFile(new URL('./components/ImageDrawingEditor.tsx',import.meta.url),'utf8'),ast=parse(sourceCode,{sourceType:'module',plugins:['typescript','jsx']});
  const body=ast.program.body.find(node=>node.type==='ExportDefaultDeclaration').declaration.body.body;
  const command=body.find(node=>node.type==='VariableDeclaration'&&node.declarations[0].id.name==='command');
  const port=body.find(node=>node.type==='VariableDeclaration'&&node.declarations[0].id.name==='port').declarations[0].init;
  const allows=port.properties.find(property=>property.key.name==='allows').value;
  const actual=new SourceTextModule(stripTypeScriptTypes(`export function fixture(props){${sourceCode.slice(command.start,command.end)};return{command,allows:${sourceCode.slice(allows.start,allows.end)}};}`,{mode:'transform'}));
  await actual.link(()=>{throw Error('No runtime dependency');});await actual.evaluate();
  const calls=[],value=item(),adapter=actual.namespace.fixture({sceneId:scene,item:value,onCommand:async action=>{calls.push(plain(action));}});
  const action={type:'remove_drawings',drawingIds:['ink-a','ink-b'],expectedRevision:7};
  assert.equal(adapter.allows(action),true);await adapter.command(action);
  assert.deepEqual(calls,[{type:'edit_image_annotations',sceneId:scene,itemId:id,sourceAssetId:source,expectedRevision:7,action:{type:'remove_drawings',drawingIds:['ink-a','ink-b']}}]);
  assert.deepEqual(value,item());assert.equal(calls.some(value=>value.type==='remove_item'),false);
});
