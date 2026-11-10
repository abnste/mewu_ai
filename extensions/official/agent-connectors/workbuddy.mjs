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
// Desktop product catalogs are refreshed independently of the bundled CLI.
// Pass the desktop-owned file to that CLI; never copy account credentials.
export function desktopProductEnvironment({fs,path,root,cli,env,isAlive}){
 if(['ACC_PRODUCT_CONFIG_PATH','ACC_PRODUCT_CONFIG_V3','ACC_PRODUCT_CONFIG_V2','ACC_PRODUCT_CONFIG'].some(key=>env[key]?.trim()))return {};
 const readProduct=file=>{
  try{
   const stat=fs.lstatSync(file);
   if(!stat.isFile()||stat.isSymbolicLink()||stat.size>4*1024*1024)return null;
   const value=JSON.parse(fs.readFileSync(file,'utf8'));
   return value.productName==='WorkBuddy'&&Array.isArray(value.models)?stat:null;
  }catch{return null;}
 };
 const cache=path.join(root,'cache');let entries=[];
 try{
  if(!fs.lstatSync(cache).isSymbolicLink())entries=fs.readdirSync(cache).flatMap(name=>{
   const match=/^acc-product-config-v3-(\d+)-[a-f0-9-]+\.json$/i.exec(name);
   if(!match||!isAlive(Number(match[1])))return [];
   const file=path.join(cache,name),stat=readProduct(file);
   return stat?[{file,time:stat.mtimeMs}]:[];
  });
 }catch{}
 entries.sort((a,b)=>b.time-a.time||a.file.localeCompare(b.file));
 const file=entries[0]?.file??path.resolve(path.dirname(cli),'..','product.json');
 return readProduct(file)?{ACC_PRODUCT_CONFIG_PATH:file}:{};
}
export const bootstrap=String.raw`
const http=require('node:http'),net=require('node:net'),readline=require('node:readline'),{spawn}=require('node:child_process');
const fs=require('node:fs'),os=require('node:os'),path=require('node:path'),crypto=require('node:crypto');
${desktopProductEnvironment.toString()}
const config=JSON.parse(process.argv[1]);let child,port,connection,stopped=false,managed,control;
let leaseServer;const leaseSockets=new Set();
const closeLease=()=>{for(const socket of leaseSockets)socket.destroy();leaseServer?.close();};
async function createLease(){
 const token=crypto.randomBytes(32).toString('hex');
 leaseServer=net.createServer(socket=>{
  leaseSockets.add(socket);socket.on('close',()=>leaseSockets.delete(socket));socket.on('error',()=>{});
  socket.setTimeout(5000,()=>socket.destroy());let input='';
  socket.on('data',chunk=>{input+=chunk;if(input.length>128)return socket.destroy();if(input.includes('\n')){if(input!==token+'\n'||stopped)return socket.destroy();socket.setTimeout(0);socket.write('ok\n');}});
 });
 await new Promise((resolve,reject)=>{leaseServer.once('error',reject);leaseServer.listen(0,'127.0.0.1',resolve);});
 if(stopped){closeLease();throw Error('Connection closed');}
 return {port:leaseServer.address().port,token};
}
// Run the bundled CLI in this process, so even a forced bridge termination
// closes its lifetime lease and exits the desktop-owned runtime immediately.
function managedEntrypoint(){
 const net=require('node:net'),{pathToFileURL}=require('node:url');
 const value=JSON.parse(process.argv[1]);let started=false,buffer='';
 const socket=net.connect({host:'127.0.0.1',port:value.lease.port});
 socket.setTimeout(5000,()=>process.exit(1));socket.on('error',()=>process.exit(started?0:1));socket.on('close',()=>process.exit(0));
 socket.on('connect',()=>socket.write(value.lease.token+'\n'));
 socket.on('data',chunk=>{if(started)return;buffer+=chunk;if(buffer.length>32)process.exit(1);if(buffer==='ok\n'){
  started=true;socket.setTimeout(0);process.argv=[process.execPath,value.cli,...value.args];
  import(pathToFileURL(value.cli).href).catch(()=>process.exit(1));
 }});
}
const calls=new Set();
const desktopCall=(method,params={})=>new Promise((resolve,reject)=>{
 const socket=net.connect(control);let buffer='',settled=false;
 const finish=(error,value)=>{if(settled)return;settled=true;socket.destroy();error?reject(error):resolve(value);};
 socket.setTimeout(60000,()=>finish(Error('Desktop control timeout')));
 socket.on('error',error=>finish(error));socket.on('close',()=>finish(Error('Desktop control closed')));
 socket.on('connect',()=>socket.write(JSON.stringify({jsonrpc:'2.0',id:1,method,params})+'\n'));
 socket.setEncoding('utf8');socket.on('data',chunk=>{buffer+=chunk;if(buffer.length>1048576)return finish(Error('Desktop control response limit'));let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at);buffer=buffer.slice(at+1);try{const value=JSON.parse(line);if(value.id===1)finish(value.error?Error('Desktop control request failed'):null,value.result);}catch{finish(Error('Invalid desktop control response'));}}});
});
async function startManagedDesktop(){
 const root=path.resolve(process.env.WORKBUDDY_CONFIG_DIR?.trim()||process.env.CODEBUDDY_CONFIG_DIR?.trim()||path.join(os.homedir(),'.workbuddy'));
 const hash=(value,length)=>crypto.createHash('sha1').update(value).digest('hex').slice(0,length);
 const instance=hash(root,12),uid=process.getuid?.();
 const bases=process.platform==='linux'?[process.env.XDG_RUNTIME_DIR&&path.join(process.env.XDG_RUNTIME_DIR,'workbuddy',instance),uid!==undefined&&path.join('/run/user',String(uid),'workbuddy',instance)]:[];
 bases.push(path.join(os.tmpdir(),uid===undefined?'wb':'wb-'+hash(String(uid),6),instance));
 let record,base;for(const candidate of bases.filter(Boolean)){try{record=JSON.parse(fs.readFileSync(path.join(candidate,'sidecar.pid'),'utf8'));base=candidate;break;}catch(error){if(error.code!=='ENOENT')throw Error('Invalid desktop runtime discovery');}}
 if(!record)return false;
 if(record.version<6||!Number.isInteger(record.pid)||typeof record.token!=='string'||!record.token||!/^[a-f0-9]{8}$/.test(record.controlPipeUuid??''))throw Error('Unsupported desktop runtime');
 control=process.platform==='win32'?'\\\\.\\pipe\\workbuddy-'+instance+'-sidecar-control-'+record.controlPipeUuid:path.join(base,'sidecar-'+record.controlPipeUuid+'.sock');
 const ping=await desktopCall('sidecar.ping');
 if(ping?.pid!==record.pid||ping.token!==record.token||ping.version!==record.version)throw Error('Desktop runtime identity mismatch');
 if(stopped)return false;
 // Let the logged-in desktop sidecar launch the CLI. Its normal readiness
 // handshake delivers the credential bootstrap; the plugin never reads keys.
 const sessionId=crypto.randomUUID();
 managed={sessionId};
 const lease=await createLease();
 const args=['--eval','('+managedEntrypoint.toString()+')()',JSON.stringify({cli:config.cli,args:[...config.args.filter(arg=>arg!=='--acp'),'--serve','--host','127.0.0.1'],lease})];
 const productEnv=desktopProductEnvironment({fs,path,root,cli:config.cli,env:process.env,isAlive:pid=>{try{process.kill(pid,0);return true;}catch{return false;}}});
 const result=await desktopCall('session.create',{sessionId,command:process.execPath,args,cwd:process.cwd(),env:{...process.env,...productEnv,CODEBUDDY_CONFIG_DIR:root,WORKBUDDY_CONFIG_DIR:root,...(record.epochAddress?{SANDBOX_CENTER_IPC_ADDRESS:record.epochAddress}:{})},port:0,cols:80,rows:24});
 if(result?.sessionId!==sessionId||!Number.isInteger(result.pid)||typeof result.runtimeId!=='string')throw Error('Invalid desktop session');
 managed.runtimeId=result.runtimeId;
 const endpoint=new URL(result.acpEndpoint);
 if(endpoint.protocol!=='http:'||endpoint.hostname!=='127.0.0.1'||endpoint.pathname!=='/api/v1/acp'||!endpoint.port)throw Error('Invalid desktop endpoint');
 port=Number(endpoint.port);return true;
}
const output=value=>{if(!stopped)process.stdout.write(JSON.stringify(value)+'\n');};
const request=(method,path,body,onData)=>new Promise((resolve,reject)=>{
 const bytes=body===undefined?undefined:Buffer.from(JSON.stringify(body));
 const call=http.request({host:'127.0.0.1',port,path,method,agent:false,headers:{'x-codebuddy-request':'1',...(connection?{'acp-connection-id':connection}:{}),...(bytes?{'content-type':'application/json','content-length':bytes.length}:{}),accept:'application/json, text/event-stream'}},response=>{
  const abort=message=>{reject(Error(message));response.destroy();call.destroy();};
  if(response.statusCode<200||response.statusCode>=300){response.resume();reject(Error('WorkBuddy HTTP '+response.statusCode));return;}
  // Newer desktop versions return unary JSON for some ACP methods. Only
  // event-stream responses should go through the SSE parser.
  if(onData&&!/^application\/json\b/i.test(response.headers['content-type']??'')){let buffer='',total=0;response.setEncoding('utf8');response.on('data',chunk=>{total+=Buffer.byteLength(chunk);buffer+=chunk;if(total>33554432||buffer.length>2097152){abort('WorkBuddy response limit');return;}let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at).trimEnd();buffer=buffer.slice(at+1);if(line.startsWith('data:')){try{onData(JSON.parse(line.slice(5).trim()));}catch{abort('Invalid WorkBuddy event');return;}}}});response.on('end',()=>{if(buffer.startsWith('data:')){try{onData(JSON.parse(buffer.slice(5).trim()));}catch{reject(Error('Invalid WorkBuddy event'));return;}}resolve();});}
  else{let result='',total=0;response.setEncoding('utf8');response.on('data',chunk=>{total+=Buffer.byteLength(chunk);if(total>1048576){abort('WorkBuddy response limit');return;}result+=chunk;});response.on('end',()=>{try{const value=result?JSON.parse(result):null;if(onData&&value!==null)onData(value);resolve(value);}catch{reject(Error('Invalid WorkBuddy response'));}});}
  response.on('error',reject);response.on('aborted',()=>reject(Error('WorkBuddy stream closed')));
 });
 calls.add(call);call.on('close',()=>calls.delete(call));call.on('error',reject);call.setTimeout(260000,()=>{reject(Error('WorkBuddy request timeout'));call.destroy();});if(bytes)call.write(bytes);call.end();
});
const ready=(async()=>{
 if(await startManagedDesktop()){
  const value=await request('POST','/api/v1/acp/connect');
  if(typeof value?.connectionId!=='string'||!value.connectionId)throw Error('Invalid desktop connection');
  connection=value.connectionId;return;
 }
 if(stopped)return;
 const listener=net.createServer();await new Promise((resolve,reject)=>{listener.once('error',reject);listener.listen(0,'127.0.0.1',resolve);});port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
 child=spawn(process.execPath,[config.cli,...config.args.filter(arg=>arg!=='--acp'),'--serve','--host','127.0.0.1','--port',String(port)],{cwd:process.cwd(),env:process.env,stdio:['pipe','pipe','pipe'],windowsHide:true});
 child.stdout.resume();child.stderr.resume();let exited=false;child.once('exit',()=>{exited=true;});child.once('error',()=>{exited=true;});
 const until=Date.now()+24000;while(Date.now()<until&&!exited&&!stopped){try{const value=await request('POST','/api/v1/acp/connect');if(typeof value?.connectionId==='string'&&value.connectionId){connection=value.connectionId;return;}}catch{}await new Promise(resolve=>setTimeout(resolve,150));}
 throw Error('WorkBuddy desktop connection failed');
})();ready.catch(()=>{closeLease();});
const input=readline.createInterface({input:process.stdin,crlfDelay:Infinity});
input.on('line',line=>{let value;try{if(Buffer.byteLength(line)>16777216)throw Error();value=JSON.parse(line);}catch{process.exitCode=1;input.close();return;}
 void(async()=>{await ready;let answered=false;await request('POST','/api/v1/acp',value,message=>{if(value.method!==undefined&&message.id===value.id&&Object.hasOwn(message,'result')||message.id===value.id&&Object.hasOwn(message,'error'))answered=true;output(message);});if(value.method!==undefined&&value.id!==undefined&&!answered)throw Error('WorkBuddy response missing');})().catch(()=>{if(value.id!==undefined&&value.method!==undefined)output({jsonrpc:'2.0',id:value.id,error:{code:-32603,message:'WorkBuddy desktop request failed'}});});
});
input.on('close',()=>{stopped=true;closeLease();for(const call of calls)call.destroy();child?.kill();if(managed){void ready.catch(()=>{}).then(()=>managed.runtimeId?desktopCall('session.kill',{sessionId:managed.sessionId,expectedRuntimeId:managed.runtimeId}):undefined).catch(()=>{}).finally(()=>process.exit(0));setTimeout(()=>process.exit(0),65000).unref();}else setTimeout(()=>process.exit(0),200).unref();});
`;
