// SPDX-License-Identifier: MPL-2.0
// Real OS-pipe fixture. No network, credentials or user data.
const fs=require('node:fs'),path=require('node:path'),readline=require('node:readline');
fs.writeFileSync(path.join(process.argv[2],'started.json'),JSON.stringify({pid:process.pid}));
readline.createInterface({input:process.stdin}).on('line',line=>{
  const input=JSON.parse(line);
  if(input.method==='partial'){
    const bytes=Buffer.from(JSON.stringify({id:input.id,result:{text:'中文🙂'}})+'\n');
    const index=bytes.indexOf(Buffer.from('中'))+1;
    process.stdout.write(bytes.subarray(0,index));setTimeout(()=>process.stdout.write(bytes.subarray(index)),120);
  }else process.stdout.write(JSON.stringify({id:input.id,result:input.params})+'\n');
});
