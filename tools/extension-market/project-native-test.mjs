// SPDX-License-Identifier: MPL-2.0
// This suite must use the current production validator; missing/stale is failure.
import assert from 'node:assert/strict';
import test from 'node:test';
import { deflateSync } from 'node:zlib';
import { createValidator, projectFingerprint } from './validator.mjs';
import { crc32 } from './zip.mjs';
import { sha256 } from './github.mjs';
if (!process.env.MEWU_PLUGIN_VALIDATOR) throw new Error('需要当前真实 MEWU_PLUGIN_VALIDATOR，不允许跳过项目原生门');
const validate=await createValidator(process.env.MEWU_PLUGIN_VALIDATOR),encoded=value=>Buffer.from(JSON.stringify(value));
const chunk=(name,data)=>{const type=Buffer.from(name),result=Buffer.alloc(data.length+12);result.writeUInt32BE(data.length);type.copy(result,4);data.copy(result,8);result.writeUInt32BE(crc32(Buffer.concat([type,data])),data.length+8);return result;};
const header=Buffer.alloc(13);header.writeUInt32BE(16);header.writeUInt32BE(16,4);header[8]=8;header[9]=6;
const signature=Buffer.from([137,80,78,71,13,10,26,10]);
const image=Buffer.concat([signature,chunk('IHDR',header),chunk('IDAT',deflateSync(Buffer.alloc(16*(1+16*4)))),chunk('IEND',Buffer.alloc(0))]);
const project={id:'thirdparty.gallery',version:'1.0.0',packageSha256:'a'.repeat(64),sourceRepository:'author/gallery',sourceCommit:'b'.repeat(40),screenshots:[{path:'screenshots/real.png',size:image.length,sha256:sha256(image),width:16,height:16,caption:'真实格式合成像素'}]};
test('actual native project parser accepts immutable project and no-screenshot projects independently of the plugin catalog',async()=>{
 assert.deepEqual(await validate('extension-project',encoded(project)),project);
 assert.deepEqual((await validate('extension-projects',encoded({schemaVersion:1,projects:[{...project,screenshots:[]}]}))).projects[0].screenshots,[]);
 assert.match(await projectFingerprint(),/^[a-f0-9]{64}$/);
});
test('actual raw parser rejects unknown/duplicate fields, moving refs, source takeover and path/raster bounds',async()=>{
 await assert.rejects(validate('extension-project',Buffer.from(`{"id":"other",${JSON.stringify(project).slice(1)}`)));
 for(const change of [{extra:true},{sourceCommit:'main'},{sourceRepository:'author/..'},{sourceRepository:'https://github.com/author/gallery'},{version:'latest'},
  {screenshots:[{...project.screenshots[0],path:'../secret.png'}]},{screenshots:[{...project.screenshots[0],path:'x.svg'}]},
  {screenshots:[{...project.screenshots[0],width:4097}]},{screenshots:[{...project.screenshots[0],caption:'line\nbreak'}]}]) await assert.rejects(validate('extension-project',encoded({...project,...change})));
 await assert.rejects(validate('extension-projects',encoded({schemaVersion:1,projects:[project,{...project,id:'another.plugin'}]})));
 await assert.rejects(validate('extension-projects',encoded({schemaVersion:2,projects:[project]})));
});
test('actual image decoder verifies pixels, exact SHA/dimensions and strict image input before returning metadata',async()=>{
 const input={project,index:0,data:image.toString('base64')};const result=await validate('extension-project-image',encoded(input));assert.equal(result.width,16);assert.equal(result.sha256,sha256(image));
 await assert.rejects(validate('extension-project-image',encoded({...input,index:1})));
 await assert.rejects(validate('extension-project-image',encoded({...input,extra:true})));
 await assert.rejects(validate('extension-project-image',encoded({...input,project:{...project,screenshots:[{...project.screenshots[0],sha256:'0'.repeat(64)}]}})));
 await assert.rejects(validate('extension-project-image',encoded({...input,project:{...project,screenshots:[{...project.screenshots[0],width:32}]}})));
 const broken=Buffer.concat([signature,chunk('IHDR',header),chunk('IDAT',Buffer.from('not a zlib stream')),chunk('IEND',Buffer.alloc(0))]);
 await assert.rejects(validate('extension-project-image',encoded({project:{...project,screenshots:[{...project.screenshots[0],sha256:sha256(broken),size:broken.length}]},index:0,data:broken.toString('base64')})));
});
