// SPDX-License-Identifier: MPL-2.0
#![cfg(target_os = "linux")]

#[path = "../src/recording_codec_linux.rs"]
mod recording_codec_linux;
#[path = "../src/recording_aac_linux.rs"]mod recording_aac_linux;
#[path = "../src/recording_mp4_audio_linux.rs"]mod recording_mp4_audio_linux;

use gst::prelude::*;
use gstreamer as gst;
use std::{
    fs::OpenOptions,
    os::unix::fs::OpenOptionsExt,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn element(name: &str) -> gst::Element {
    gst::ElementFactory::make(name).build().unwrap()
}

#[test]
#[ignore = "Requires official native GStreamer/OpenH264 SDK; synthetic frames only"]
fn linux_native_fixed_mp4_roundtrip() {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("mewu-codec-{}-{id}.mp4", std::process::id()));
    struct Output(std::path::PathBuf);
    impl Drop for Output {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let output_file = Output(path.clone());
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    let mut encoder =
        recording_codec_linux::Codec::new(file, 64, 48, Instant::now() + Duration::from_secs(10))
            .unwrap();
    let bytes = 64usize * 48 * 4;
    for index in 0..30 {
        if index == 15 {
            std::thread::sleep(Duration::from_millis(150));
        }
        let mut frame = vec![0; bytes];
        for pixel in frame.chunks_exact_mut(4) {
            pixel.copy_from_slice(if index < 15 {
                &[255, 0, 0, 255]
            } else {
                &[0, 255, 0, 255]
            });
        }
        encoder.push_rgba(frame, &|| true).unwrap();
    }
    assert_eq!(encoder.finish(&|| true).unwrap(), 30);
    assert!(std::fs::metadata(&path).unwrap().len() > 1000);

    let decoder = gst::Pipeline::new();
    struct NativeDecoder(gst::Pipeline);
    impl Drop for NativeDecoder {
        fn drop(&mut self) {
            let _ = self.0.set_state(gst::State::Null);
        }
    }
    let guard = NativeDecoder(decoder.clone());
    let input = element("filesrc");
    input.set_property("location", path.to_str().unwrap());
    let demux = element("qtdemux");
    let parse = element("h264parse");
    let byte_stream = element("capsfilter");
    byte_stream.set_property(
        "caps",
        gst::Caps::builder("video/x-h264")
            .field("stream-format", "byte-stream")
            .field("alignment", "au")
            .build(),
    );
    let decode = element("openh264dec");
    let convert = element("videoconvert");
    let rgba = element("capsfilter");
    rgba.set_property(
        "caps",
        gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .build(),
    );
    let output = element("appsink")
        .downcast::<gstreamer_app::AppSink>()
        .unwrap();
    output.set_max_buffers(2);
    output.set_property("sync", false);
    let nodes: [&gst::Element; 8] = [
        &input,
        &demux,
        &parse,
        &byte_stream,
        &decode,
        &convert,
        &rgba,
        output.upcast_ref(),
    ];
    decoder.add_many(nodes).unwrap();
    input.link(&demux).unwrap();
    gst::Element::link_many([
        &parse,
        &byte_stream,
        &decode,
        &convert,
        &rgba,
        output.upcast_ref(),
    ])
    .unwrap();
    let target = parse.static_pad("sink").unwrap();
    demux.connect_pad_added(move |_, pad| {
        assert!(!target.is_linked(), "unexpected additional MP4 stream");
        pad.link(&target).unwrap();
    });
    decoder.set_state(gst::State::Playing).unwrap();
    let expires = Instant::now() + Duration::from_secs(10);
    let mut count = 0u64;
    while !output.is_eos() {
        assert!(Instant::now() < expires, "native decoded MP4 timeout");
        if let Some(sample) = output.try_pull_sample(gst::ClockTime::from_mseconds(20)) {
            let caps = sample.caps().unwrap().structure(0).unwrap();
            assert_eq!(caps.get::<i32>("width").unwrap(), 64);
            assert_eq!(caps.get::<i32>("height").unwrap(), 48);
            let buffer = sample.buffer().unwrap();
            let actual_pts = buffer.pts().unwrap().nseconds();
            let expected_pts = count * 1_000_000_000 / 15;
            assert!(
                actual_pts.abs_diff(expected_pts) < 1_000_000,
                "output timestamps include synthetic producer pause"
            );
            let pixels = buffer.map_readable().unwrap();
            assert_eq!(pixels.len(), bytes);
            for pixel in pixels.as_slice().chunks_exact(4) {
                if count < 15 {
                    assert!(pixel[0] > 220 && pixel[1] < 35 && pixel[2] < 35);
                } else {
                    assert!(pixel[1] > 220 && pixel[0] < 35 && pixel[2] < 35);
                }
            }
            count += 1;
        }
        if let Some(message) = decoder
            .bus()
            .unwrap()
            .pop_filtered(&[gst::MessageType::Error])
        {
            if let gst::MessageView::Error(error) = message.view() {
                panic!("native decoder: {}", error.error());
            }
        }
    }
    assert_eq!(count, 30);
    decoder.set_state(gst::State::Null).unwrap();
    let (result, state, _) = decoder.state(gst::ClockTime::ZERO);
    result.unwrap();
    assert_eq!(state, gst::State::Null);
    drop(guard);
    drop(decoder);
    drop(output_file);
    println!("NATIVE_SYNTHETIC_MP4_PASS frames=30 decoded_caps=64x48 colors=verified pts=continuous producer_pause_ms=150 appsrc_bound=2 audio=not-connected pipeline_null=verified");
}
