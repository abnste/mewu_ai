// SPDX-License-Identifier: MPL-2.0
// Transport/filesystem tests use explicit API/parser doubles; native parser and
// image decoding are exercised separately by project-native-test.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { deflateSync } from 'node:zlib';
import { buildProjects, checkPngCrc, sourceReader, verifyProject } from './projects.mjs';
import { crc32, createZip } from './zip.mjs';
import { sha256 } from './github.mjs';
import { collectPullRequest } from './collect-pr.mjs';
import { invokeExtensionValidator } from './validator.mjs';

const encoded = value => Buffer.from(JSON.stringify(value));
const blobHash = bytes => createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex');
function png() {
  const chunk = (kind, body) => { const type=Buffer.from(kind), result=Buffer.alloc(body.length+12);result.writeUInt32BE(body.length);type.copy(result,4);body.copy(result,8);result.writeUInt32BE(crc32(Buffer.concat([type,body])),body.length+8);return result; };
  const header=Buffer.alloc(13);header.writeUInt32BE(16);header.writeUInt32BE(16,4);header[8]=8;header[9]=6;
  return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(Buffer.alloc(16*(1+16*4)))),chunk('IEND',Buffer.alloc(0))]);
}
const image=png(), sourceCommit='a'.repeat(40), treeSha='b'.repeat(40);
const manifest={schemaVersion:2,id:'thirdparty.gallery',version:'1.0.0',name:'Gallery',description:'Own test',tags:['tools'],license:'MPL-2.0',hostApi:2,runtime:'javascript',entry:'index.mjs',permissions:[]};
const source=Buffer.from('export default ctx=>{};'), manifestBytes=encoded(manifest);
const archive=createZip([{name:'mewu-plugin.json',bytes:manifestBytes},{name:'index.mjs',bytes:source}]);
const entry={id:manifest.id,name:manifest.name,description:manifest.description,tags:manifest.tags,version:manifest.version,repository:'market/packages',releaseTag:'v1.0.0',assetName:'gallery.zip',sha256:sha256(archive),size:archive.length,hostApi:2,platforms:['any']};
const project={id:manifest.id,version:manifest.version,packageSha256:entry.sha256,sourceRepository:'author/gallery',sourceCommit,screenshots:[{path:'screenshots/real.png',sha256:sha256(image),size:image.length,width:16,height:16}]};
const sourceFiles=new Map([['mewu-plugin.json',manifestBytes],['index.mjs',source],['LICENSE',Buffer.from('Own synthetic license')],['screenshots/real.png',image]]);
const response=value=>new Response(encoded(value),{headers:{'content-type':'application/json'}});
function network(files=sourceFiles, mutate=()=>{}) {
  const nodes=[...files].map(([name,bytes])=>({path:name,type:'blob',mode:'100644',sha:blobHash(bytes),size:bytes.length}));mutate(nodes);
  const calls=[];
  const fetchImpl=async url=>{
    const endpoint=String(url);calls.push(endpoint);
    if(endpoint.endsWith(`/git/commits/${sourceCommit}`))return response({sha:sourceCommit,tree:{sha:treeSha}});
    if(endpoint.endsWith(`/git/trees/${treeSha}?recursive=1`))return response({sha:treeSha,truncated:false,tree:nodes});
    const file=[...files].find(([,bytes])=>endpoint.endsWith(`/git/blobs/${blobHash(bytes)}`));
    if(file)return response({sha:blobHash(file[1]),encoding:'base64',content:file[1].toString('base64')});
    if(endpoint.endsWith('/releases/tags/v1.0.0'))return response({id:1,tag_name:'v1.0.0',draft:false,assets:[{id:2,state:'uploaded',name:entry.assetName,size:archive.length,digest:`sha256:${entry.sha256}`,url:'https://api.github.com/repos/market/packages/releases/assets/2',browser_download_url:'https://github.com/market/packages/releases/download/v1.0.0/gallery.zip'}]});
    if(endpoint.endsWith('/releases/assets/2'))return new Response(archive);
    throw Error('Unexpected endpoint');
  };
  return {fetchImpl,calls};
}
const parserDouble=async (_mode,bytes)=>JSON.parse(bytes.toString('utf8'));
async function temporary(t){const directory=await mkdtemp(path.join(tmpdir(),'mewu-project-tools-'));t.after(()=>rm(directory,{recursive:true,force:true}));return directory;}

test('immutable source transport pins commit/tree/blob and never reads executable content as a command',async()=>{
  const api=network(), read=await sourceReader(project.sourceRepository,sourceCommit,api);
  assert.deepEqual(await read('index.mjs'),source);
  assert.ok(api.calls.every(url=>url.startsWith('https://api.github.com/repos/author/gallery/git/')));
  assert.ok(api.calls.some(url=>url.includes(`/commits/${sourceCommit}`)));
  for(const mode of ['120000','160000']){const read=await sourceReader(project.sourceRepository,sourceCommit,network(sourceFiles,nodes=>{nodes.find(node=>node.path==='index.mjs').mode=mode;}));await assert.rejects(read('index.mjs'),/普通文件/);}
  await assert.rejects(sourceReader('https://attacker.test/project',sourceCommit,api),/固定 GitHub/);
  await assert.rejects(sourceReader(project.sourceRepository,'main',api),/完整 Git/);
});

test('source transport rejects duplicate tree paths and bytes not matching the committed blob',async()=>{
  await assert.rejects(sourceReader(project.sourceRepository,sourceCommit,network(sourceFiles,nodes=>nodes.push(nodes[0]))),/路径重复/);
  const api=network(), read=await sourceReader(project.sourceRepository,sourceCommit,{fetchImpl:async(url,options)=>String(url).includes('/git/blobs/')?response({sha:blobHash(source),encoding:'base64',content:'dGFtcGVyZWQ='}):api.fetchImpl(url,options)});
  await assert.rejects(read('index.mjs'),/字节与固定/);
});

test('bounded source reader accepts GitHubs wrapped base64 at the declared raw file limit',async()=>{
  const bytes=Buffer.alloc(128*1024,65),files=new Map([['index.mjs',bytes]]),api=network(files);
  const read=await sourceReader(project.sourceRepository,sourceCommit,{fetchImpl:async(url,options)=>String(url).includes('/git/blobs/')
    ?response({sha:blobHash(bytes),encoding:'base64',content:bytes.toString('base64').match(/.{1,60}/g).join('\n')+'\n'}):api.fetchImpl(url,options)});
  assert.deepEqual(await read('index.mjs',bytes.length),bytes);
});

test('project verifies actual ZIP modules and forwards bounded raster bytes to native decoding',async()=>{
  let decoded=0;
  const validate=async(mode,bytes)=>{const value=JSON.parse(bytes);if(mode==='extension-project-image'){decoded++;assert.deepEqual(Buffer.from(value.data,'base64'),image);assert.equal(value.index,0);assert.equal(value.project.sourceCommit,sourceCommit);return {};}return value;};
  const result=await verifyProject(project,entry,validate,network());
  assert.equal(result.sourceAndPackageBytesMatched,true);assert.equal(result.modules.length,2);assert.equal(result.screenshots.length,1);assert.equal(decoded,1);
  const changed=new Map(sourceFiles);changed.set('index.mjs',Buffer.from('different code'));
  await assert.rejects(verifyProject(project,entry,validate,network(changed)),/源码与真实发行 ZIP/);
  await assert.rejects(verifyProject({...project,packageSha256:'0'.repeat(64)},entry,validate,network()),/安装包 SHA/);
});

test('PNG CRC and ending bytes are verified before any native decoder call',()=>{
  checkPngCrc(image);
  const damaged=Buffer.from(image);damaged[45]^=1;assert.throws(()=>checkPngCrc(damaged),/CRC32/);
  assert.throws(()=>checkPngCrc(Buffer.concat([image,Buffer.from('extra')])),/结束块/);
  assert.throws(()=>checkPngCrc(image.subarray(0,20)),/截断/);
});

test('sidecar generation is deterministic, keeps canonical bytes intact and preserves previous sidecar on verification failure',async t=>{
  const directory=await temporary(t),input=path.join(directory,'projects'),out=path.join(directory,'mewu-projects.json'),catalog=path.join(directory,'mewu-catalog.json');await mkdir(input);
  const canonical=encoded({schemaVersion:2,plugins:[entry]});await writeFile(catalog,canonical);await writeFile(path.join(input,`${project.id}.json`),encoded({...project,screenshots:[]}));
  const options={input,out,catalog};const verify=async()=>({verified:true});
  const result=await buildProjects(options,parserDouble,verify);assert.equal(result.projectsVerified,true);
  const before=await readFile(out);assert.equal((await buildProjects({...options,check:true},parserDouble,verify)).checked,true);
  await assert.rejects(buildProjects(options,parserDouble,async()=>{throw Error('Own network failure');}),/network failure/);
  assert.deepEqual(await readFile(out),before);assert.deepEqual(await readFile(catalog),canonical);
  await writeFile(path.join(input,`${project.id}.json`),encoded({...project,sourceRepository:'new/owner'}));
  await assert.rejects(buildProjects(options,parserDouble,verify),/不能更换/);assert.deepEqual(await readFile(out),before);
});

test('sidecar refuses unbound versions, output inside inputs and mismatched file identity',async t=>{
  const directory=await temporary(t),input=path.join(directory,'projects'),out=path.join(directory,'out.json'),catalog=path.join(directory,'catalog.json');await mkdir(input);await writeFile(catalog,encoded({schemaVersion:2,plugins:[entry]}));
  await writeFile(path.join(input,`${project.id}.json`),encoded({...project,version:'2.0.0'}));
  await assert.rejects(buildProjects({input,out,catalog},parserDouble),/目录版本/);
  await assert.rejects(buildProjects({input,out:path.join(input,'out.json'),catalog},parserDouble),/输出不能/);
});

test('community CI requires an independent source project for every installed catalog entry',async t=>{
  const directory=await temporary(t),input=path.join(directory,'projects'),out=path.join(directory,'out.json'),catalog=path.join(directory,'catalog.json');await mkdir(input);await writeFile(catalog,encoded({schemaVersion:2,plugins:[entry]}));
  await assert.rejects(buildProjects({input,out,catalog},parserDouble),/每个社区插件/);
});

test('PR sidecars are fixed Git blobs only; author workflow changes and source symlinks are refused',async t=>{
  const directory=await temporary(t),entryBytes=encoded(entry),projectBytes=encoded(project),head='c'.repeat(40);
  const data=[['entries',entryBytes],['projects',projectBytes]].map(([folder,bytes])=>({folder,bytes,sha:blobHash(bytes)}));
  const event={number:1,repository:{full_name:'market/catalog'},pull_request:{head:{sha:head}}};
  const pr={state:'open',changed_files:2,base:{repo:{full_name:'market/catalog'}},head:{sha:head,repo:{full_name:'author/catalog'}}};
  let changed=data.map(item=>({filename:`${item.folder}/${entry.id}.json`,status:'added'})),mode='100644';
  const fetchImpl=async url=>{const value=String(url);if(value.endsWith('/pulls/1'))return response(pr);if(value.includes('/files?'))return response(changed);if(value.includes('/git/trees/'))return response({truncated:false,tree:data.map(item=>({path:`${item.folder}/${entry.id}.json`,type:'blob',mode,sha:item.sha,size:item.bytes.length}))});const item=data.find(item=>value.endsWith(`/git/blobs/${item.sha}`));return response({sha:item.sha,encoding:'base64',content:item.bytes.toString('base64')});};
  const result=await collectPullRequest({event,repository:'market/catalog',output:path.join(directory,'entries'),projectOutput:path.join(directory,'projects')},{fetchImpl});assert.equal(result.entries,1);assert.equal(result.projects,1);assert.deepEqual(await readFile(path.join(directory,'projects',`${entry.id}.json`)),projectBytes);
  changed=[{filename:'.github/workflows/run-author-code.yml',status:'modified'}];pr.changed_files=1;
  await assert.rejects(collectPullRequest({event,repository:'market/catalog',output:path.join(directory,'bad-entry'),projectOutput:path.join(directory,'bad-project')},{fetchImpl}),/只能修改/);
  changed=data.map(item=>({filename:`${item.folder}/${entry.id}.json`,status:'added'}));pr.changed_files=2;mode='120000';
  await assert.rejects(collectPullRequest({event,repository:'market/catalog',output:path.join(directory,'link-entry'),projectOutput:path.join(directory,'link-project')},{fetchImpl}),/链接/);
});

test('project validator protocol rejects stale project fingerprint and API2-only producer',async t=>{
  const directory=await temporary(t),script=path.join(directory,'producer.mjs');
  await writeFile(script,'process.stdin.resume();process.stdin.on("end",()=>console.log(JSON.stringify({protocolVersion:1,projectContractVersion:1,projectContractSha256:"new-project",ok:true,value:{projects:[]}})));');
  assert.deepEqual(await invokeExtensionValidator(process.execPath,script,Buffer.from('{}'),'new-project',15000,true),{projects:[]});
  await assert.rejects(invokeExtensionValidator(process.execPath,script,Buffer.from('{}'),'stale',15000,true),/契约/);
  await writeFile(script,'console.log(JSON.stringify({protocolVersion:1,extensionContractVersion:2,extensionContractSha256:"new-project",ok:true,value:{projects:[]}}));');
  await assert.rejects(invokeExtensionValidator(process.execPath,script,Buffer.from('{}'),'new-project',15000,true),/契约/);
});
