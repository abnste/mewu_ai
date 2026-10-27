// SPDX-License-Identifier: MPL-2.0
// Desktop WorkBuddy exposes HTTP ACP. This plugin-owned stdio bridge uses the
// already installed client's Node runtime; no helper or Node is added to Mewu.
export function workbuddyProcess(config){
  if(config.transport!=='workbuddy-http')return config;
  if(!config.args?.[0])throw Error('请选择 WorkBuddy 内置 CLI 入口');
  // The desktop client's embedded Node cannot load its entry script through a
  // Win32 namespace prefix (realpathSync fails at the drive root). Preserve the
  // resolved executable; use the equivalent ordinary drive path for the script.
  const cli=config.args[0].replace(/^\\\\\?\\(?=[A-Za-z]:\\)/,'');
  return {...config,args:['--eval',`eval(${JSON.stringify(bootstrap)})`,JSON.stringify({cli,args:config.args.slice(1)})]};
}
export const bootstrap=String.raw`
const http=require('node:http'),net=require('node:net'),readline=require('node:readline'),{spawn}=require('node:child_process');
const config=JSON.parse(process.argv[1]);let child,port,connection,stopped=false;
const calls=new Set();
const output=value=>{if(!stopped)process.stdout.write(JSON.stringify(value)+'\n');};
const request=(method,path,body,onData)=>new Promise((resolve,reject)=>{
 const bytes=body===undefined?undefined:Buffer.from(JSON.stringify(body));
 const call=http.request({host:'127.0.0.1',port,path,method,agent:false,headers:{'x-codebuddy-request':'1',...(connection?{'acp-connection-id':connection}:{}),...(bytes?{'content-type':'application/json','content-length':bytes.length}:{}),accept:'application/json, text/event-stream'}},response=>{
  const abort=message=>{reject(Error(message));response.destroy();call.destroy();};
  if(response.statusCode<200||response.statusCode>=300){response.resume();reject(Error('WorkBuddy HTTP '+response.statusCode));return;}
  if(onData){let buffer='',total=0;response.setEncoding('utf8');response.on('data',chunk=>{total+=Buffer.byteLength(chunk);buffer+=chunk;if(total>33554432||buffer.length>2097152){abort('WorkBuddy response limit');return;}let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at).trimEnd();buffer=buffer.slice(at+1);if(line.startsWith('data:')){try{onData(JSON.parse(line.slice(5).trim()));}catch{abort('Invalid WorkBuddy event');return;}}}});response.on('end',()=>{if(buffer.startsWith('data:')){try{onData(JSON.parse(buffer.slice(5).trim()));}catch{reject(Error('Invalid WorkBuddy event'));return;}}resolve();});}
  else{let result='',total=0;response.setEncoding('utf8');response.on('data',chunk=>{total+=Buffer.byteLength(chunk);if(total>1048576){abort('WorkBuddy response limit');return;}result+=chunk;});response.on('end',()=>{try{resolve(result?JSON.parse(result):null);}catch{reject(Error('Invalid WorkBuddy response'));}});}
  response.on('error',reject);response.on('aborted',()=>reject(Error('WorkBuddy stream closed')));
 });
 calls.add(call);call.on('close',()=>calls.delete(call));call.on('error',reject);call.setTimeout(260000,()=>{reject(Error('WorkBuddy request timeout'));call.destroy();});if(bytes)call.write(bytes);call.end();
});
const ready=(async()=>{
 const listener=net.createServer();await new Promise((resolve,reject)=>{listener.once('error',reject);listener.listen(0,'127.0.0.1',resolve);});port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 child=spawn(process.execPath,[config.cli,...config.args.filter(arg=>arg!=='--acp'),'--serve','--host','127.0.0.1','--port',String(port)],{cwd:process.cwd(),env:process.env,stdio:['pipe','pipe','pipe'],windowsHide:true});
 child.stdout.resume();child.stderr.resume();let exited=false;child.once('exit',()=>{exited=true;});child.once('error',()=>{exited=true;});
 const until=Date.now()+24000;while(Date.now()<until&&!exited&&!stopped){try{const value=await request('POST','/api/v1/acp/connect');if(typeof value?.connectionId==='string'&&value.connectionId){connection=value.connectionId;return;}}catch{}await new Promise(resolve=>setTimeout(resolve,150));}
 throw Error('WorkBuddy desktop connection failed');
})();ready.catch(()=>{});
const input=readline.createInterface({input:process.stdin,crlfDelay:Infinity});
input.on('line',line=>{let value;try{if(Buffer.byteLength(line)>16777216)throw Error();value=JSON.parse(line);}catch{process.exitCode=1;input.close();return;}
 void(async()=>{await ready;let answered=false;await request('POST','/api/v1/acp',value,message=>{if(value.method!==undefined&&message.id===value.id&&Object.hasOwn(message,'result')||message.id===value.id&&Object.hasOwn(message,'error'))answered=true;output(message);});if(value.method!==undefined&&value.id!==undefined&&!answered)throw Error('WorkBuddy response missing');})().catch(()=>{if(value.id!==undefined&&value.method!==undefined)output({jsonrpc:'2.0',id:value.id,error:{code:-32603,message:'WorkBuddy desktop request failed'}});});
});
input.on('close',()=>{stopped=true;for(const call of calls)call.destroy();child?.kill();setTimeout(()=>process.exit(0),200).unref();});
`;
