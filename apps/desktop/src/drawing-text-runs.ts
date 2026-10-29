// SPDX-License-Identifier: MPL-2.0
/** Literal fragment wire: closed fields, exact source text and bounded effects. */
export function validDrawingTextRuns(text:unknown,value:unknown):boolean {
  if(value===undefined)return true;
  if(typeof text!=='string'||!Array.isArray(value)||!value.length||value.length>Math.min(2000,[...text].length))return false;
  let offset=0;
  for(const run of value){
    if(!run||typeof run!=='object'||Array.isArray(run)||Object.keys(run).some(key=>!['text','color','textStyle'].includes(key))||typeof run.text!=='string'||!run.text||text.slice(offset,offset+run.text.length)!==run.text)return false;
    // Rust strings contain Unicode scalars; reject split/unpaired UTF-16 too.
    if([...run.text].some(character=>{const point=character.codePointAt(0)!;return point>=0xD800&&point<=0xDFFF;}))return false;
    if(run.color!==undefined&&(typeof run.color!=='string'||!/^#[\da-f]{6}$/i.test(run.color)))return false;
    if(run.textStyle!==undefined&&(!run.textStyle||typeof run.textStyle!=='object'||Array.isArray(run.textStyle)||Object.entries(run.textStyle).some(([key,value])=>!['bold','italic','underline','strikethrough'].includes(key)||typeof value!=='boolean')))return false;
    offset+=run.text.length;
  }
  return offset===text.length;
}
