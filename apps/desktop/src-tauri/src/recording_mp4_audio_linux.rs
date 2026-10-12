// SPDX-License-Identifier: MPL-2.0
//! Fixed qtmux output only: update its existing single audio edit-list duration
//! after real EOS. No box insertion, chunk relocation or guessed AAC priming.
use std::{fs::File,os::unix::fs::FileExt};
#[derive(Clone,Copy)]struct BoxRef{kind:[u8;4],body:u64,end:u64}
fn read<const N:usize>(file:&File,offset:u64)->Result<[u8;N],String>{let mut value=[0;N];file.read_exact_at(&mut value,offset).map_err(|_|"无法核对MP4结构")?;Ok(value)}
fn boxes(file:&File,start:u64,end:u64)->Result<Vec<BoxRef>,String>{
    let mut next=start;let mut out=Vec::new();while next<end{
        if out.len()>=128||end-next<8{return Err("MP4结构无效".into());}
        let header=read::<8>(file,next)?;let mut size=u32::from_be_bytes(header[..4].try_into().unwrap())as u64;
        let body=if size==1{if end-next<16{return Err("MP4结构无效".into());}
            size=u64::from_be_bytes(read::<8>(file,next+8)?);next+16}else{next+8};
        if size<body-next||size>end-next{return Err("MP4结构无效".into());}
        out.push(BoxRef{kind:header[4..8].try_into().unwrap(),body,end:next+size});next+=size;
    }Ok(out)
}
fn one(values:&[BoxRef],kind:[u8;4])->Result<BoxRef,String>{let mut found=values.iter().filter(|b|b.kind==kind);
    let value=found.next().copied().ok_or("MP4必需结构缺失")?;if found.next().is_some(){return Err("MP4结构重复".into());}Ok(value)}
pub(crate) fn finalize_edit(file:&File,pcm:u64,delay:u32,encoded:u64)->Result<(),String>{
    if pcm==0||pcm>48000*600||encoded<pcm+delay as u64||encoded>pcm+delay as u64+16384{return Err("AAC尾点覆盖无效".into());}
    let size=file.metadata().map_err(|_|"无法核对MP4大小")?.len();if size>512*1024*1024{return Err("MP4超出预算".into());}
    let top=boxes(file,0,size)?;let moov=one(&top,*b"moov")?;if moov.end-moov.body>16*1024*1024{return Err("MP4元数据超出预算".into());}
    let children=boxes(file,moov.body,moov.end)?;let movie=one(&children,*b"mvhd")?;
    if movie.end-movie.body<24||read::<4>(file,movie.body)?!=[0,0,0,0]
        ||u32::from_be_bytes(read::<4>(file,movie.body+12)?)!=48000{return Err("MP4时间基准无效".into());}
    let mut audio=None;let mut count=0;
    for track in children.iter().filter(|b|b.kind==*b"trak"){
        count+=1;let track_boxes=boxes(file,track.body,track.end)?;let mdia=one(&track_boxes,*b"mdia")?;
        let media=boxes(file,mdia.body,mdia.end)?;let handler=one(&media,*b"hdlr")?;
        if read::<4>(file,handler.body+8)?!=*b"soun"{continue;}
        if audio.is_some(){return Err("MP4音轨重复".into());}
        let header=one(&track_boxes,*b"tkhd")?;let clock=one(&media,*b"mdhd")?;
        if read::<4>(file,clock.body)?!=[0,0,0,0]||u32::from_be_bytes(read::<4>(file,clock.body+12)?)!=48000
            ||u32::from_be_bytes(read::<4>(file,clock.body+16)?)as u64!=encoded{return Err("AAC实际样本时长不匹配".into());}
        let edit=one(&track_boxes,*b"edts")?;let entries=boxes(file,edit.body,edit.end)?;let list=one(&entries,*b"elst")?;
        if list.end-list.body!=20||read::<4>(file,list.body)?!=[0,0,0,0]
            ||u32::from_be_bytes(read::<4>(file,list.body+4)?)!=1
            ||u32::from_be_bytes(read::<4>(file,list.body+12)?)!=0
            ||read::<4>(file,list.body+16)?!=[0,1,0,0]{return Err("AAC真实初始化延迟未写入MP4".into());}
        if header.end-header.body<84||read::<1>(file,header.body)?!=[0]{return Err("MP4音轨头无效".into());}
        audio=Some((header,list));
    }
    if count!=2{return Err("MP4轨道集合无效".into());}
    let (header,list)=audio.ok_or("MP4音轨缺失")?;let duration=(pcm as u32).to_be_bytes();
    file.write_all_at(&duration,list.body+8).map_err(|_|"无法保存AAC尾点")?;
    file.write_all_at(&delay.to_be_bytes(),list.body+12).map_err(|_|"无法保存AAC真实初始化延迟")?;
    file.write_all_at(&duration,header.body+20).map_err(|_|"无法保存音轨时长")?;
    file.write_all_at(&duration,movie.body+16).map_err(|_|"无法保存MP4时长")?;
    Ok(())
}
