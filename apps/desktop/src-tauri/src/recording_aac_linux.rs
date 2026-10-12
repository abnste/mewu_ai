// SPDX-License-Identifier: MPL-2.0
//! Fixed LC/stereo encoder. Delay and codec configuration come from the real
//! initialized encoder; end-of-input is explicitly drained to native EOF.
use fdk_aac_sys as fdk;
use std::{ffi::c_void,ptr};
pub(crate) struct Encoder{handle:fdk::HANDLE_AACENCODER,pub(crate) info:fdk::AACENC_InfoStruct,frames:u64}
impl Drop for Encoder{fn drop(&mut self){unsafe{fdk::aacEncClose(&mut self.handle);}}}
impl Encoder{
    pub(crate) fn new()->Result<Self,String>{unsafe{
        let mut handle=ptr::null_mut();
        if fdk::aacEncOpen(&mut handle,0,2)!=fdk::AACENC_ERROR_AACENC_OK||handle.is_null(){return Err("无法创建AAC编码器".into());}
        let mut encoder=Self{handle,info:std::mem::zeroed(),frames:0};
        for (parameter,value) in [(fdk::AACENC_PARAM_AACENC_AOT,2),(fdk::AACENC_PARAM_AACENC_SAMPLERATE,48000),
            (fdk::AACENC_PARAM_AACENC_CHANNELMODE,2),(fdk::AACENC_PARAM_AACENC_CHANNELORDER,1),
            (fdk::AACENC_PARAM_AACENC_BITRATE,160000),(fdk::AACENC_PARAM_AACENC_TRANSMUX,0)]{
            if fdk::aacEncoder_SetParam(handle,parameter,value)!=fdk::AACENC_ERROR_AACENC_OK{return Err("无法配置AAC编码器".into());}
        }
        if fdk::aacEncEncode(handle,ptr::null(),ptr::null(),ptr::null(),ptr::null_mut())!=fdk::AACENC_ERROR_AACENC_OK
            ||fdk::aacEncInfo(handle,&mut encoder.info)!=fdk::AACENC_ERROR_AACENC_OK{
            return Err("无法启动AAC编码器".into());}
        if encoder.info.inputChannels!=2||encoder.info.frameLength==0||encoder.info.frameLength>4096
            ||encoder.info.nDelayCore>16384||encoder.info.maxOutBufBytes==0||encoder.info.maxOutBufBytes>16384
            ||encoder.info.confSize==0||encoder.info.confSize>64{return Err("AAC编码配置无效".into());}
        Ok(encoder)
    }}
    pub(crate) fn configuration(&self)->&[u8]{&self.info.confBuf[..self.info.confSize as usize]}
    fn encode(&mut self,input:Option<&[i16]>)->Result<(usize,Vec<u8>,bool),String>{unsafe{
        if input.is_some_and(|v|v.len()>self.info.frameLength as usize*2||v.len()%2!=0){return Err("AAC输入帧长度无效".into());}
        let mut buffer=vec![0u8;self.info.maxOutBufBytes as usize];let mut output_ptr=buffer.as_mut_ptr().cast::<c_void>();
        let mut out_id=fdk::AACENC_BufferIdentifier_OUT_BITSTREAM_DATA as i32;let mut out_size=buffer.len()as i32;let mut out_element=1;
        let out=fdk::AACENC_BufDesc{numBufs:1,bufs:&mut output_ptr,bufferIdentifiers:&mut out_id,bufSizes:&mut out_size,bufElSizes:&mut out_element};
        let empty=[];let samples=input.unwrap_or(&empty);let mut input_ptr=samples.as_ptr().cast_mut().cast::<c_void>();
        let mut in_id=fdk::AACENC_BufferIdentifier_IN_AUDIO_DATA as i32;let mut in_size=(samples.len()*2)as i32;let mut in_element=2;
        let input_desc=fdk::AACENC_BufDesc{numBufs:1,bufs:&mut input_ptr,bufferIdentifiers:&mut in_id,bufSizes:&mut in_size,bufElSizes:&mut in_element};
        let args=fdk::AACENC_InArgs{numInSamples:if input.is_some(){samples.len()as i32}else{-1},numAncBytes:0};
        let mut result:fdk::AACENC_OutArgs=std::mem::zeroed();
        let code=fdk::aacEncEncode(self.handle,&input_desc,&out,&args,&mut result);
        let ended=code==fdk::AACENC_ERROR_AACENC_ENCODE_EOF;
        if code!=fdk::AACENC_ERROR_AACENC_OK&&!ended{return Err("AAC编码失败".into());}
        if result.numOutBytes<0||result.numOutBytes as usize>buffer.len()||result.numInSamples<0
            ||result.numInSamples as usize>samples.len(){return Err("AAC编码回执无效".into());}
        buffer.truncate(result.numOutBytes as usize);
        if !buffer.is_empty(){self.frames+=1;if self.frames>32000{return Err("AAC编码帧超出上限".into());}}
        Ok((result.numInSamples as usize,buffer,ended))
    }}
    pub(crate) fn feed(&mut self,mut input:&[i16],mut output:impl FnMut(Vec<u8>)->Result<(),String>)->Result<(),String>{
        let mut calls=0;
        while !input.is_empty(){calls+=1;if calls>128{return Err("AAC输入处理无进展".into());}
            let window=input.len().min(self.info.frameLength as usize*2);let (consumed,encoded,ended)=self.encode(Some(&input[..window]))?;
            if ended||consumed==0{return Err("AAC输入处理无进展".into());}
            input=&input[consumed..];if !encoded.is_empty(){output(encoded)?;}
        }Ok(())
    }
    pub(crate) fn drain(&mut self,mut output:impl FnMut(Vec<u8>)->Result<(),String>)->Result<(),String>{
        for _ in 0..32{let (_,encoded,ended)=self.encode(None)?;if !encoded.is_empty(){output(encoded)?;}if ended{return Ok(());}}
        Err("AAC尾帧未能结束".into())
    }
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn recording_wayland_aac_uses_native_delay_and_drains_fractional_tail(){
        let mut encoder=Encoder::new().unwrap();let delay=encoder.info.nDelayCore;
        assert!(!encoder.configuration().is_empty());let length=encoder.info.frameLength;
        let samples:Vec<i16>=(0..96037).flat_map(|i|{let value=((i as f64*440.*std::f64::consts::TAU/48000.).sin()*12000.)as i16;[value,value]}).collect();
        let mut encoded=0u64;for chunk in samples.chunks(1920){encoder.feed(chunk,|_|{encoded+=1;Ok(())}).unwrap();}
        let before=encoded;encoder.drain(|_|{encoded+=1;Ok(())}).unwrap();
        assert!(encoded>before,"native drain must retain real pending AAC output");
        assert!(encoded*length as u64>=96037+delay as u64);
        println!("FDK actual delay={delay} frameLength={length} inputFrames=96037 encodedPackets={encoded} nativeEof=true");
    }
}
