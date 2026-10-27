// SPDX-License-Identifier: MPL-2.0
// Real Node processes, pipes and loopback HTTP/SSE; the server is an explicit
// protocol fixture, not a claim that a particular WorkBuddy login works.
import test from 'node:test';
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {openPeer} from '../../../extensions/official/agent-connectors/rpc.mjs';

test('plugin-owned WorkBuddy transport bridges real JSON lines to bounded loopback ACP SSE and preserves server replies',async()=>{
  const temp=await mkdtemp(path.join(tmpdir(),'mewu-wb-'));
  const script=path.join(temp,'fixture.cjs');
  await writeFile(script,String.raw`
const http=require('node:http');
const args=process.argv.slice(2),port=Number(args[args.indexOf('--port')+1]);
if(args[args.indexOf('--host')+1]!=='127.0.0.1')process.exit(2);
let waiting;
http.createServer(async(req,res)=>{
 if(req.headers['x-codebuddy-request']!=='1'){res.writeHead(403);res.end();return;}
 if(req.url==='/api/v1/acp/connect'){res.setHeader('content-type','application/json');res.end(JSON.stringify({connectionId:'test-only'}));return;}
 if(req.url!=='/api/v1/acp'||req.headers['acp-connection-id']!=='test-only'){res.writeHead(404);res.end();return;}
 let body='';for await(const part of req)body+=part;const value=JSON.parse(body);
 res.setHeader('content-type','text/event-stream');
 const event=message=>res.write('data: '+JSON.stringify({jsonrpc:'2.0',...message})+'\n\n');
 if(value.method==='initialize'){event({id:value.id,result:{protocolVersion:1}});res.end();}
 else if(value.method==='prompt'){
  event({method:'session/update',params:{sessionId:'one',update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text:'中文✓'}}}});
  waiting=()=>{event({id:value.id,result:{stopReason:'end_turn'}});res.end();};
  event({id:'permission',method:'session/request_permission',params:{options:[{kind:'reject_once',optionId:'deny'}]}});
 }else if(value.id==='permission'){
  if(value.result?.outcome?.optionId!=='deny'){res.writeHead(400);res.end();return;}
  res.end();waiting();
 }else if(value.method==='malformed'){res.end('data: invalid-json\n\n');}
}).listen(port,'127.0.0.1');
`);
  let child,exit,closed=false,buffer='',stderr='';const queue=[];
  const ctx={processes:{
    open:async config=>{assert.ok(config.args.every(arg=>!/[\u0000-\u001f]/.test(arg)),'fixed-program transport arguments must not contain control characters');assert.ok(config.executable.length+config.args.join('').length<16384);child=spawn(config.executable,config.args,{cwd:temp,stdio:['pipe','pipe','pipe'],windowsHide:true});exit=once(child,'exit');child.stderr.on('data',chunk=>{stderr+=chunk;});child.stdout.setEncoding('utf8');child.stdout.on('data',chunk=>{buffer+=chunk;let at;while((at=buffer.indexOf('\n'))>=0){queue.push(JSON.parse(buffer.slice(0,at)));buffer=buffer.slice(at+1);}});return{id:'owned',cwd:temp};},
    write:async(id,message)=>{assert.equal(id,'owned');child.stdin.write(JSON.stringify(message)+'\n');},
    read:async()=>{const until=Date.now()+250;while(!queue.length&&Date.now()<until){if(child.exitCode!==null)throw Error('fixture closed');await new Promise(r=>setTimeout(r,5));}return queue.shift()??null;},
    close:async()=>{if(closed)return;closed=true;child.stdin.end();await Promise.race([exit,new Promise((_,reject)=>{const timer=setTimeout(()=>reject(Error('bridge did not stop')),4000);timer.unref();})]);}
  }};
  let peer;
  try{
    peer=await openPeer(ctx,{executable:process.execPath,args:[script],transport:'workbuddy-http'});
    assert.equal((await peer.request('initialize',{})).protocolVersion,1);
    const events=[];peer.notification=async(method,params)=>events.push({method,params});
    assert.equal((await peer.request('prompt',{})).stopReason,'end_turn');
    assert.equal(events[0].params.update.content.text,'中文✓');
    await assert.rejects(peer.request('malformed',{}),error=>{
      assert.match(error.publicMessage,/模型的权限及官方客户端的服务状态/);
      assert.doesNotMatch(error.message,/登录|WorkBuddy desktop request failed/);
      return true;
    });
    await peer.close();
  }finally{
    if(!closed&&child){child.stdin.end();await exit;}
    if(child?.exitCode)console.error(stderr);
    assert.equal(path.dirname(temp),path.resolve(tmpdir()));assert.ok(path.basename(temp).startsWith('mewu-wb-'));
    await rm(temp,{recursive:true});
  }
});
