// SPDX-License-Identifier: MPL-2.0
//! WASAPI owners live only in the supervised recording child. No device is
//! enumerated or activated for Mute. MFT conversion and mux calls never run while
//! holding an endpoint's GetBuffer, or a host/renderer state lock.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum AudioMode {
    #[default]
    Mute,
    System,
    Microphone,
    Both,
}
impl AudioMode {
    pub fn enabled(self) -> bool {
        self != Self::Mute
    }
    fn sources(self) -> &'static [bool] {
        match self {
            Self::Mute => &[],
            Self::System => &[true],
            Self::Microphone => &[false],
            Self::Both => &[true, false],
        }
    }
}
impl From<AudioMode> for String {
    fn from(v: AudioMode) -> Self {
        match v {
            AudioMode::Mute => "mute",
            AudioMode::System => "system",
            AudioMode::Microphone => "microphone",
            AudioMode::Both => "both",
        }
        .into()
    }
}
impl TryFrom<String> for AudioMode {
    type Error = String;
    fn try_from(v: String) -> Result<Self, String> {
        match v.as_str() {
            "mute" => Ok(Self::Mute),
            "system" => Ok(Self::System),
            "microphone" => Ok(Self::Microphone),
            "both" => Ok(Self::Both),
            _ => Err("录屏声音模式无效".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum RecordingStopReason {
    AudioDeviceChanged,
    AudioUnavailable,
    AudioOverrun,
    AudioTimingInvalid,
}
impl From<RecordingStopReason> for String {
    fn from(v: RecordingStopReason) -> Self {
        match v {
            RecordingStopReason::AudioDeviceChanged => "audio_device_changed",
            RecordingStopReason::AudioUnavailable => "audio_unavailable",
            RecordingStopReason::AudioOverrun => "audio_overrun",
            RecordingStopReason::AudioTimingInvalid => "audio_timing_invalid",
        }
        .into()
    }
}
impl TryFrom<String> for RecordingStopReason {
    type Error = String;
    fn try_from(v: String) -> Result<Self, String> {
        match v.as_str() {
            "audio_device_changed" => Ok(Self::AudioDeviceChanged),
            "audio_unavailable" => Ok(Self::AudioUnavailable),
            "audio_overrun" => Ok(Self::AudioOverrun),
            "audio_timing_invalid" => Ok(Self::AudioTimingInvalid),
            _ => Err("录屏停止原因无效".into()),
        }
    }
}

#[cfg(windows)]
pub use platform::{qpc_now, AudioEngine, StartError};

#[cfg(windows)]
pub(crate) mod platform {
    use super::*;
    use crate::recording_audio_timeline::{
        self as time, Mixer, Timeline, TimingError, SAMPLE_RATE,
    };
    use std::{
        collections::VecDeque,
        mem::ManuallyDrop,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::{self, Receiver},
            Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };
    use windows::{
        core::Interface,
        Win32::{
            Media::{Audio::*, MediaFoundation::*},
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
                    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
                },
                Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
            },
        },
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_FAILED},
        System::Threading::{CreateEventW, WaitForSingleObject},
    };

    const MAX_RAW_BYTES: usize = 2 * 1024 * 1024; // per source, at most two sources
    const MAX_RAW_PACKETS: usize = 128;
    const MAX_RAW_TICKS: u64 = 5_000_000;
    const MAX_SINGLE_PACKET: usize = 1024 * 1024;
    const TIMESTAMP_GRACE: u64 = 200_000;

    pub fn qpc_now() -> Result<u64, String> {
        let (mut counter, mut frequency) = (0, 0);
        unsafe {
            QueryPerformanceFrequency(&mut frequency).map_err(|_| "无法读取录制时钟")?;
            QueryPerformanceCounter(&mut counter).map_err(|_| "无法读取录制时钟")?;
        }
        if counter < 0 || frequency <= 0 {
            return Err("录制时钟无效".into());
        }
        u64::try_from((counter as u128) * 10_000_000 / frequency as u128)
            .map_err(|_| "录制时钟溢出".into())
    }

    struct Apartment;
    impl Apartment {
        fn new() -> Result<Self, String> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED)
                    .ok()
                    .map_err(|_| "无法初始化录音线程")?;
            }
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    struct Event(HANDLE);
    impl Event {
        fn new() -> Result<Self, String> {
            let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
            if handle.is_null() {
                Err("无法创建录音事件".into())
            } else {
                Ok(Self(handle))
            }
        }
    }
    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    struct FormatAllocation(*mut WAVEFORMATEX);
    impl Drop for FormatAllocation {
        fn drop(&mut self) {
            unsafe {
                CoTaskMemFree(Some(self.0.cast()));
            }
        }
    }
    struct StopClient(IAudioClient);
    impl Drop for StopClient {
        fn drop(&mut self) {
            unsafe {
                let _ = self.0.Stop();
            }
        }
    }
    struct ReleasePacket<'a> {
        capture: &'a IAudioCaptureClient,
        frames: u32,
    }
    impl Drop for ReleasePacket<'_> {
        fn drop(&mut self) {
            unsafe {
                let _ = self.capture.ReleaseBuffer(self.frames);
            }
        }
    }

    #[derive(Clone, Debug)]
    pub(crate) struct Format {
        pub rate: u32,
        pub channels: u16,
        pub bits: u16,
        pub valid_bits: u16,
        pub mask: u32,
        pub float: bool,
    }
    impl Format {
        fn align(&self) -> usize {
            self.channels as usize * self.bits as usize / 8
        }
        fn validate(&self) -> Result<(), String> {
            if !(8_000..=192_000).contains(&self.rate)
                || !(1..=8).contains(&self.channels)
                || ![16, 24, 32].contains(&self.bits)
                || self.valid_bits == 0
                || self.valid_bits > self.bits
                || (self.float && (self.bits != 32 || self.valid_bits != 32))
                || self.mask.count_ones() != self.channels as u32
            {
                return Err("当前声音设备的格式暂不支持".into());
            }
            Ok(())
        }
        unsafe fn read(pointer: *const WAVEFORMATEX) -> Result<Self, String> {
            if pointer.is_null() {
                return Err("声音设备未返回格式".into());
            }
            let wave = unsafe { pointer.read_unaligned() };
            let (float, valid_bits, mask) = match wave.wFormatTag {
                1 | 3 if wave.cbSize == 0 => (
                    wave.wFormatTag == 3,
                    wave.wBitsPerSample,
                    match wave.nChannels {
                        1 => 4,
                        2 => 3,
                        _ => return Err("声音设备缺少声道布局".into()),
                    },
                ),
                0xfffe if wave.cbSize >= 22 => {
                    let extended =
                        unsafe { pointer.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
                    let subtype = extended.SubFormat;
                    let float = if subtype == MFAudioFormat_PCM {
                        false
                    } else if subtype == MFAudioFormat_Float {
                        true
                    } else {
                        return Err("当前声音设备的编码暂不支持".into());
                    };
                    (
                        float,
                        unsafe { extended.Samples.wValidBitsPerSample },
                        extended.dwChannelMask,
                    )
                }
                _ => return Err("当前声音设备的格式暂不支持".into()),
            };
            let format = Self {
                rate: wave.nSamplesPerSec,
                channels: wave.nChannels,
                bits: wave.wBitsPerSample,
                valid_bits,
                mask,
                float,
            };
            format.validate()?;
            if wave.nBlockAlign as usize != format.align()
                || wave.nAvgBytesPerSec != format.rate * format.align() as u32
            {
                return Err("声音设备格式长度无效".into());
            }
            Ok(format)
        }
    }

    #[derive(Debug)]
    struct Packet {
        qpc: u64,
        frames: usize,
        discontinuity: bool,
        bytes: Vec<u8>,
    }
    #[derive(Default)]
    struct Queue {
        packets: VecDeque<Packet>,
        bytes: usize,
        armed: bool,
        failure: Option<RecordingStopReason>,
    }
    impl Queue {
        fn push(&mut self, packet: Packet) {
            if self.failure.is_some() {
                return;
            }
            let too_full = |q: &Self| {
                q.bytes + packet.bytes.len() > MAX_RAW_BYTES
                    || q.packets.len() >= MAX_RAW_PACKETS
                    || q.packets
                        .front()
                        .is_some_and(|first| packet.qpc.saturating_sub(first.qpc) > MAX_RAW_TICKS)
            };
            if self.armed && too_full(self) {
                self.failure = Some(RecordingStopReason::AudioOverrun);
                return;
            }
            while !self.armed && too_full(self) {
                let Some(old) = self.packets.pop_front() else {
                    break;
                };
                self.bytes -= old.bytes.len();
            }
            self.bytes += packet.bytes.len();
            self.packets.push_back(packet);
        }
    }
    struct Source {
        queue: Arc<Mutex<Queue>>,
        stop: Arc<AtomicBool>,
        join: Option<JoinHandle<()>>,
        ready: Receiver<Result<Format, String>>,
    }
    impl Source {
        fn spawn(system: bool) -> Result<Self, String> {
            let queue = Arc::new(Mutex::new(Queue::default()));
            let stop = Arc::new(AtomicBool::new(false));
            let (send, ready) = mpsc::sync_channel(1);
            let (worker_queue, worker_stop) = (queue.clone(), stop.clone());
            let join = thread::Builder::new()
                .name("mewu-record-audio".into())
                .spawn(move || {
                    let result = capture(system, &worker_stop, &worker_queue, &send);
                    if let Err(error) = result {
                        let _ = send.try_send(Err(match error {
                            RecordingStopReason::AudioDeviceChanged => "声音设备已改变",
                            _ => "无法访问所选声音设备，请检查设备和麦克风权限",
                        }
                        .into()));
                        worker_queue
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .failure
                            .get_or_insert(error);
                    }
                })
                .map_err(|_| "无法创建录音线程")?;
            Ok(Self {
                queue,
                stop,
                join: Some(join),
                ready,
            })
        }
        fn stop(&mut self) {
            self.stop.store(true, Ordering::Release);
        }
        fn join(&mut self) {
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }
    impl Drop for Source {
        fn drop(&mut self) {
            self.stop();
            self.join();
        }
    }

    fn device_id(device: &IMMDevice) -> Result<Vec<u16>, RecordingStopReason> {
        let pointer =
            unsafe { device.GetId() }.map_err(|_| RecordingStopReason::AudioDeviceChanged)?;
        if pointer.0.is_null() {
            return Err(RecordingStopReason::AudioDeviceChanged);
        }
        let result = (|| {
            let mut len = 0;
            while len < 4096 && unsafe { *pointer.0.add(len) } != 0 {
                len += 1;
            }
            if len == 4096 {
                Err(RecordingStopReason::AudioDeviceChanged)
            } else {
                Ok(unsafe { std::slice::from_raw_parts(pointer.0, len) }.to_vec())
            }
        })();
        unsafe {
            CoTaskMemFree(Some(pointer.0.cast()));
        }
        result
    }

    #[derive(Default)]
    struct PacketClock {
        previous: Option<(u64, u64, usize)>,
        bad_ticks: u64,
    }
    impl PacketClock {
        fn resolve(
            &mut self,
            device: u64,
            qpc: u64,
            frames: usize,
            flags: u32,
            rate: u32,
            now: u64,
        ) -> Result<u64, RecordingStopReason> {
            let stamp = if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
                let (old_device, old_qpc, old_frames) = self
                    .previous
                    .ok_or(RecordingStopReason::AudioTimingInvalid)?;
                if device != old_device.saturating_add(old_frames as u64) {
                    return Err(RecordingStopReason::AudioTimingInvalid);
                }
                self.bad_ticks = self
                    .bad_ticks
                    .saturating_add(time::frames_to_ticks(frames as u64, rate));
                if self.bad_ticks > TIMESTAMP_GRACE {
                    return Err(RecordingStopReason::AudioTimingInvalid);
                }
                old_qpc.saturating_add(time::frames_to_ticks(old_frames as u64, rate))
            } else {
                self.bad_ticks = 0;
                qpc
            };
            if stamp > now.saturating_add(TIMESTAMP_GRACE)
                || now.saturating_sub(stamp) > MAX_RAW_TICKS
            {
                return Err(RecordingStopReason::AudioTimingInvalid);
            }
            if self
                .previous
                .is_some_and(|(_, previous, _)| stamp < previous)
            {
                return Err(RecordingStopReason::AudioTimingInvalid);
            }
            self.previous = Some((device, stamp, frames));
            Ok(stamp)
        }
    }

    fn capture(
        system: bool,
        stop: &AtomicBool,
        queue: &Mutex<Queue>,
        ready: &mpsc::SyncSender<Result<Format, String>>,
    ) -> Result<(), RecordingStopReason> {
        fn unavailable<E>(_: E) -> RecordingStopReason {
            RecordingStopReason::AudioUnavailable
        }
        let _apartment = Apartment::new().map_err(unavailable)?;
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER) }
                .map_err(unavailable)?;
        let flow = if system { eRender } else { eCapture };
        let device =
            unsafe { enumerator.GetDefaultAudioEndpoint(flow, eConsole) }.map_err(unavailable)?;
        let identity = device_id(&device)?;
        let client: IAudioClient =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(unavailable)?;
        let allocated = FormatAllocation(unsafe { client.GetMixFormat() }.map_err(unavailable)?);
        let format = unsafe { Format::read(allocated.0) }.map_err(unavailable)?;
        let event = Event::new().map_err(unavailable)?;
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_NOPERSIST
            | if system {
                AUDCLNT_STREAMFLAGS_LOOPBACK
            } else {
                0
            };
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags,
                2_000_000,
                0,
                allocated.0,
                None,
            )
        }
        .map_err(unavailable)?;
        unsafe { client.SetEventHandle(windows::Win32::Foundation::HANDLE(event.0)) }
            .map_err(unavailable)?;
        let capture: IAudioCaptureClient = unsafe { client.GetService() }.map_err(unavailable)?;
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        unsafe { client.Start() }.map_err(unavailable)?;
        let _stop_client = StopClient(client.clone());
        let _ = ready.try_send(Ok(format.clone()));
        let mut identity_check = Instant::now();
        let mut clock = PacketClock::default();
        while !stop.load(Ordering::Acquire) {
            if unsafe { WaitForSingleObject(event.0, 10) } == WAIT_FAILED {
                return Err(RecordingStopReason::AudioUnavailable);
            }
            if identity_check.elapsed() >= Duration::from_millis(250) {
                let current = unsafe { enumerator.GetDefaultAudioEndpoint(flow, eConsole) }
                    .map_err(|_| RecordingStopReason::AudioDeviceChanged)?;
                if device_id(&current)? != identity {
                    return Err(RecordingStopReason::AudioDeviceChanged);
                }
                identity_check = Instant::now();
            }
            for _ in 0..MAX_RAW_PACKETS {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let pending = unsafe { capture.GetNextPacketSize() }
                    .map_err(|_| RecordingStopReason::AudioDeviceChanged)?;
                if pending == 0 {
                    break;
                }
                let (mut data, mut frames, mut flags, mut position, mut qpc) =
                    (std::ptr::null_mut(), 0, 0, 0, 0);
                unsafe {
                    capture.GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        Some(&mut position),
                        Some(&mut qpc),
                    )
                }
                .map_err(|_| RecordingStopReason::AudioDeviceChanged)?;
                let release = ReleasePacket {
                    capture: &capture,
                    frames,
                };
                if frames == 0 {
                    drop(release);
                    break;
                }
                let bytes = (frames as usize)
                    .checked_mul(format.align())
                    .ok_or(RecordingStopReason::AudioOverrun)?;
                if bytes > MAX_SINGLE_PACKET || frames > format.rate / 2 {
                    return Err(RecordingStopReason::AudioOverrun);
                }
                let now = qpc_now().map_err(unavailable)?;
                let qpc = clock.resolve(position, qpc, frames as usize, flags, format.rate, now)?;
                let samples = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    vec![0; bytes]
                } else {
                    if data.is_null() {
                        return Err(RecordingStopReason::AudioUnavailable);
                    }
                    unsafe { std::slice::from_raw_parts(data, bytes) }.to_vec()
                };
                drop(release); // Release endpoint ownership before queue locking.
                let mut queue = queue.lock().unwrap_or_else(|e| e.into_inner());
                queue.push(Packet {
                    qpc,
                    frames: frames as usize,
                    discontinuity: flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0,
                    bytes: samples,
                });
                if let Some(error) = queue.failure {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub(crate) unsafe fn audio_type(format: &Format) -> windows::core::Result<IMFMediaType> {
        unsafe {
            let media = MFCreateMediaType()?;
            media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            media.SetGUID(
                &MF_MT_SUBTYPE,
                if format.float {
                    &MFAudioFormat_Float
                } else {
                    &MFAudioFormat_PCM
                },
            )?;
            media.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, format.channels as u32)?;
            media.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, format.rate)?;
            media.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, format.bits as u32)?;
            media.SetUINT32(&MF_MT_AUDIO_VALID_BITS_PER_SAMPLE, format.valid_bits as u32)?;
            media.SetUINT32(&MF_MT_AUDIO_CHANNEL_MASK, format.mask)?;
            media.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, format.align() as u32)?;
            media.SetUINT32(
                &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                format.rate * format.align() as u32,
            )?;
            Ok(media)
        }
    }

    pub(crate) unsafe fn byte_sample(
        bytes: &[u8],
        timestamp: i64,
        duration: i64,
    ) -> windows::core::Result<IMFSample> {
        unsafe {
            let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
            let mut destination = std::ptr::null_mut();
            buffer.Lock(&mut destination, None, None)?;
            if destination.is_null() {
                let _ = buffer.Unlock();
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(timestamp)?;
            sample.SetSampleDuration(duration)?;
            Ok(sample)
        }
    }

    pub(crate) struct Resampler {
        transform: IMFTransform,
        format: Format,
        input_frames: u64,
    }
    impl Resampler {
        pub(crate) fn new(format: Format) -> Result<Self, String> {
            format.validate()?;
            let setup = || -> windows::core::Result<IMFTransform> {
                unsafe {
                    let transform: IMFTransform = CoCreateInstance(
                        &CLSID_AudioResamplerMediaObject,
                        None,
                        CLSCTX_INPROC_SERVER,
                    )?;
                    transform.SetInputType(0, &audio_type(&format)?, 0)?;
                    let target = Format {
                        rate: SAMPLE_RATE,
                        channels: 2,
                        bits: 32,
                        valid_bits: 32,
                        mask: 3,
                        float: true,
                    };
                    transform.SetOutputType(0, &audio_type(&target)?, 0)?;
                    if format.channels == 1 {
                        let props: IWMResamplerProps = transform.cast()?;
                        props.SetUserChannelMtx([1.0f32, 1.0].as_mut_ptr())?;
                    }
                    transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
                    transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
                    Ok(transform)
                }
            };
            Ok(Self {
                transform: setup().map_err(|_| "无法初始化声音采样转换")?,
                format,
                input_frames: 0,
            })
        }
        pub(crate) fn reset(&mut self) -> Result<(), String> {
            unsafe {
                self.transform
                    .ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0)
                    .map_err(|_| "无法重置声音采样转换")?;
                self.transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                    .map_err(|_| "无法恢复声音采样转换")?;
            }
            self.input_frames = 0;
            Ok(())
        }
        pub(crate) fn process(&mut self, bytes: &[u8]) -> Result<Vec<[f32; 2]>, String> {
            if bytes.is_empty()
                || bytes.len() % self.format.align() != 0
                || bytes.len() > MAX_SINGLE_PACKET
            {
                return Err("声音数据长度无效".into());
            }
            if self.format.float
                && bytes
                    .chunks_exact(4)
                    .any(|v| !f32::from_le_bytes(v.try_into().unwrap()).is_finite())
            {
                return Err("声音数据包含无效样本".into());
            }
            let frames = bytes.len() / self.format.align();
            let begin = time::frames_to_ticks(self.input_frames, self.format.rate);
            let end = time::frames_to_ticks(self.input_frames + frames as u64, self.format.rate);
            let sample = unsafe { byte_sample(bytes, begin as i64, (end - begin) as i64) }
                .map_err(|_| "无法创建声音样本")?;
            unsafe { self.transform.ProcessInput(0, &sample, 0) }
                .map_err(|_| "声音采样转换未接受输入")?;
            self.input_frames += frames as u64;
            self.output()
        }
        pub(crate) fn drain(&mut self) -> Result<Vec<[f32; 2]>, String> {
            unsafe {
                self.transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)
                    .map_err(|_| "无法结束声音转换")?;
                self.transform
                    .ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)
                    .map_err(|_| "无法排空声音转换")?;
            }
            self.output()
        }
        fn output(&mut self) -> Result<Vec<[f32; 2]>, String> {
            let mut result = Vec::new();
            for _ in 0..128 {
                let info = unsafe { self.transform.GetOutputStreamInfo(0) }
                    .map_err(|_| "无法读取声音转换格式")?;
                let allocated = if info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0 {
                    None
                } else {
                    let capacity = (info.cbSize as usize).max(65_536);
                    if capacity > MAX_SINGLE_PACKET || info.cbAlignment > 4096 {
                        return Err("声音转换缓冲区超限".into());
                    }
                    let sample = unsafe { MFCreateSample() }.map_err(|_| "无法分配声音样本")?;
                    let buffer =
                        unsafe { MFCreateAlignedMemoryBuffer(capacity as u32, info.cbAlignment) }
                            .map_err(|_| "无法分配声音缓冲区")?;
                    unsafe { sample.AddBuffer(&buffer) }.map_err(|_| "无法连接声音缓冲区")?;
                    Some(sample)
                };
                let mut output = MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: ManuallyDrop::new(allocated),
                    ..Default::default()
                };
                let mut status = 0;
                let call = unsafe {
                    self.transform
                        .ProcessOutput(0, std::slice::from_mut(&mut output), &mut status)
                };
                let sample = unsafe { ManuallyDrop::take(&mut output.pSample) };
                drop(unsafe { ManuallyDrop::take(&mut output.pEvents) });
                match call {
                    Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                        return Ok(result)
                    }
                    Err(_) => return Err("声音采样转换失败".into()),
                    Ok(()) => {}
                }
                let sample = sample.ok_or("声音转换未返回样本")?;
                let buffer = unsafe { sample.ConvertToContiguousBuffer() }
                    .map_err(|_| "声音转换缓冲区无效")?;
                let (mut pointer, mut length) = (std::ptr::null_mut(), 0);
                unsafe { buffer.Lock(&mut pointer, None, Some(&mut length)) }
                    .map_err(|_| "无法读取声音转换数据")?;
                let copied = if pointer.is_null()
                    || length as usize > MAX_SINGLE_PACKET
                    || length % 8 != 0
                {
                    Err("声音转换样本长度无效")
                } else {
                    let raw = unsafe { std::slice::from_raw_parts(pointer, length as usize) };
                    Ok(raw
                        .chunks_exact(8)
                        .map(|v| {
                            [
                                f32::from_le_bytes(v[..4].try_into().unwrap()),
                                f32::from_le_bytes(v[4..].try_into().unwrap()),
                            ]
                        })
                        .collect::<Vec<_>>())
                };
                let unlock = unsafe { buffer.Unlock() };
                let copied = copied?;
                unlock.map_err(|_| "无法释放声音转换数据")?;
                if copied.is_empty()
                    || copied.iter().flatten().any(|v| !v.is_finite())
                    || result.len() + copied.len() > time::MAX_PACKET_FRAMES
                {
                    return Err("声音转换数据超限或无效".into());
                }
                result.extend(copied);
            }
            Err("声音转换未在限定次数内完成".into())
        }
    }

    struct Processor {
        format: Format,
        resampler: Resampler,
        segment: Option<u64>,
        anchor: i128,
        input: u64,
        output: u64,
        previous_qpc_end: Option<u64>,
    }
    impl Processor {
        fn new(format: Format) -> Result<Self, String> {
            Ok(Self {
                resampler: Resampler::new(format.clone())?,
                format,
                segment: None,
                anchor: 0,
                input: 0,
                output: 0,
                previous_qpc_end: None,
            })
        }
        fn packet(
            &mut self,
            packet: Packet,
            timeline: &Timeline,
            mixer: &mut Mixer,
            index: usize,
        ) -> Result<(), RecordingStopReason> {
            let parts = timeline
                .parts(packet.qpc, packet.frames, self.format.rate)
                .map_err(timing_error)?;
            for part in parts {
                let discontinuity = packet.discontinuity
                    || self.segment != Some(part.segment)
                    || self
                        .previous_qpc_end
                        .is_none_or(|end| packet.qpc.abs_diff(end) > TIMESTAMP_GRACE);
                if discontinuity {
                    // Preserve the valid old segment's filter tail up to this
                    // boundary before flushing. No paused samples entered the
                    // transform, so draining cannot resurrect paused speech.
                    self.finish_into(mixer, index, part.active_ticks)?;
                    self.resampler
                        .reset()
                        .map_err(|_| RecordingStopReason::AudioUnavailable)?;
                    self.segment = Some(part.segment);
                    self.anchor = part.active_ticks as i128;
                    self.input = 0;
                    self.output = 0;
                }
                // Correct nominal resampling output to each packet's real QPC
                // anchor, so device-clock ppm does not accumulate for 10 minutes.
                let nominal =
                    self.anchor + time::frames_to_ticks(self.input, self.format.rate) as i128;
                let correction = part.active_ticks as i128 - nominal;
                if correction.abs() > time::JITTER_TICKS as i128 {
                    return Err(RecordingStopReason::AudioTimingInvalid);
                }
                self.anchor += correction;
                let timestamp =
                    self.anchor + time::frames_to_ticks(self.output, SAMPLE_RATE) as i128;
                let start = time::ticks_to_frames(
                    u64::try_from(timestamp)
                        .map_err(|_| RecordingStopReason::AudioTimingInvalid)?,
                );
                let bytes =
                    &packet.bytes[part.first * self.format.align()..part.end * self.format.align()];
                let output = self
                    .resampler
                    .process(bytes)
                    .map_err(|_| RecordingStopReason::AudioUnavailable)?;
                self.input += (part.end - part.first) as u64;
                self.output += output.len() as u64;
                mixer
                    .push(index, start, output, discontinuity)
                    .map_err(timing_error)?;
            }
            self.previous_qpc_end = Some(packet.qpc.saturating_add(time::frames_to_ticks(
                packet.frames as u64,
                self.format.rate,
            )));
            Ok(())
        }

        fn finish_into(
            &mut self,
            mixer: &mut Mixer,
            index: usize,
            until: u64,
        ) -> Result<(), RecordingStopReason> {
            if self.segment.is_none() {
                return Ok(());
            }
            let start_ticks = self.anchor + time::frames_to_ticks(self.output, SAMPLE_RATE) as i128;
            let start = time::ticks_to_frames(
                u64::try_from(start_ticks).map_err(|_| RecordingStopReason::AudioTimingInvalid)?,
            );
            let mut samples = self
                .resampler
                .drain()
                .map_err(|_| RecordingStopReason::AudioUnavailable)?;
            self.output += samples.len() as u64;
            let limit = time::ticks_to_frames(until).saturating_sub(start) as usize;
            samples.truncate(limit);
            mixer
                .push(index, start, samples, false)
                .map_err(timing_error)
        }

        fn settle_idle(
            &mut self,
            timeline: &Timeline,
            now: u64,
            mixer: &mut Mixer,
            index: usize,
        ) -> Result<(), RecordingStopReason> {
            // Loopback may provide no packets at all during silence. Drain its
            // small filter tail before the output deadline, not seconds later.
            if self.segment.is_some()
                && (timeline.paused()
                    || self
                        .previous_qpc_end
                        .is_some_and(|end| now.saturating_sub(end) >= 500_000))
            {
                self.finish_into(mixer, index, timeline.elapsed(now).map_err(timing_error)?)?;
                self.segment = None;
            }
            Ok(())
        }
    }
    fn timing_error(error: TimingError) -> RecordingStopReason {
        match error {
            TimingError::Invalid => RecordingStopReason::AudioTimingInvalid,
            TimingError::Overrun => RecordingStopReason::AudioOverrun,
        }
    }

    pub struct AudioEngine {
        sources: Vec<Source>,
        processors: Vec<Processor>,
        mixer: Mixer,
    }
    #[derive(Debug)]
    pub enum StartError {
        Stopped,
        Failed(String),
    }
    impl From<String> for StartError {
        fn from(value: String) -> Self {
            Self::Failed(value)
        }
    }
    impl From<&str> for StartError {
        fn from(value: &str) -> Self {
            Self::Failed(value.into())
        }
    }
    impl AudioEngine {
        #[cfg(test)]
        pub(crate) fn synthetic(formats: Vec<Format>) -> Result<Self, String> {
            let mixer = Mixer::new(formats.len()).map_err(|_| "合成声源数量无效")?;
            let processors = formats
                .into_iter()
                .map(Processor::new)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Self {
                sources: Vec::new(),
                processors,
                mixer,
            })
        }

        #[cfg(test)]
        pub(crate) fn synthetic_packet(
            &mut self,
            index: usize,
            qpc: u64,
            bytes: Vec<u8>,
            timeline: &Timeline,
        ) -> Result<(), RecordingStopReason> {
            let processor = self
                .processors
                .get_mut(index)
                .ok_or(RecordingStopReason::AudioUnavailable)?;
            let frames = bytes.len() / processor.format.align();
            processor.packet(
                Packet {
                    qpc,
                    frames,
                    discontinuity: false,
                    bytes,
                },
                timeline,
                &mut self.mixer,
                index,
            )
        }

        pub fn start(
            mode: AudioMode,
            mut still_allowed: impl FnMut() -> Result<bool, String>,
        ) -> Result<Option<Self>, StartError> {
            let mut check = || -> Result<(), StartError> {
                if still_allowed()? {
                    Ok(())
                } else {
                    Err(StartError::Stopped)
                }
            };
            check()?;
            if !mode.enabled() {
                return Ok(None);
            }
            let mut sources = Vec::new();
            for system in mode.sources() {
                check()?;
                sources.push(Source::spawn(*system)?);
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut processors = Vec::new();
            for source in &sources {
                let format = loop {
                    check()?;
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err("等待声音设备启动超时".into());
                    }
                    match source
                        .ready
                        .recv_timeout(remaining.min(Duration::from_millis(25)))
                    {
                        Ok(value) => break value?,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            return Err("声音采集线程已结束".into())
                        }
                    }
                };
                check()?;
                processors.push(Processor::new(format)?);
            }
            check()?;
            let mixer = Mixer::new(sources.len()).map_err(|_| "录制声源数量无效")?;
            Ok(Some(Self {
                sources,
                processors,
                mixer,
            }))
        }
        pub fn arm(&mut self) -> Result<(), String> {
            for source in &self.sources {
                let mut queue = source.queue.lock().unwrap_or_else(|e| e.into_inner());
                if queue.failure.is_some()
                    || source
                        .join
                        .as_ref()
                        .is_some_and(|thread| thread.is_finished())
                {
                    return Err("所选声音设备在录制开始前已停止".into());
                }
                queue.packets.clear();
                queue.bytes = 0;
                queue.armed = true;
            }
            Ok(())
        }
        pub fn poll(&mut self, timeline: &Timeline) -> Result<(), RecordingStopReason> {
            for (index, source) in self.sources.iter().enumerate() {
                let (packets, failure) = {
                    let mut queue = source.queue.lock().unwrap_or_else(|e| e.into_inner());
                    queue.bytes = 0;
                    (std::mem::take(&mut queue.packets), queue.failure)
                };
                for packet in packets {
                    self.processors[index].packet(packet, timeline, &mut self.mixer, index)?;
                }
                if let Some(failure) = failure {
                    return Err(failure);
                }
                if !source.stop.load(Ordering::Acquire)
                    && source
                        .join
                        .as_ref()
                        .is_some_and(|thread| thread.is_finished())
                {
                    return Err(RecordingStopReason::AudioUnavailable);
                }
            }
            self.settle_idle(
                timeline,
                qpc_now().map_err(|_| RecordingStopReason::AudioTimingInvalid)?,
            )?;
            Ok(())
        }
        pub(crate) fn settle_idle(
            &mut self,
            timeline: &Timeline,
            now: u64,
        ) -> Result<(), RecordingStopReason> {
            for (index, processor) in self.processors.iter_mut().enumerate() {
                processor.settle_idle(timeline, now, &mut self.mixer, index)?;
            }
            Ok(())
        }
        pub fn write_until(
            &mut self,
            ticks: u64,
            mut write: impl FnMut(&[i16], u64, u64) -> Result<(), String>,
        ) -> Result<(), String> {
            let until = time::ticks_to_frames(ticks);
            while self.mixer.position() < until {
                let start = self.mixer.position();
                let samples = self.mixer.next(until).ok_or("声音输出位置无效")?;
                let timestamp = time::frames_to_ticks(start, SAMPLE_RATE);
                let end = time::frames_to_ticks(self.mixer.position(), SAMPLE_RATE);
                write(&samples, timestamp, end - timestamp)?;
            }
            Ok(())
        }
        pub fn finish_processing(&mut self, until: u64) -> Result<(), RecordingStopReason> {
            for (index, processor) in self.processors.iter_mut().enumerate() {
                processor.finish_into(&mut self.mixer, index, until)?;
            }
            Ok(())
        }
        pub fn stop(&mut self) {
            for source in &mut self.sources {
                source.stop();
            }
            for source in &mut self.sources {
                source.join();
            }
        }
    }
    impl Drop for AudioEngine {
        fn drop(&mut self) {
            self.stop();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn packet_clock_extrapolation_is_short_continuous_and_bounded() {
            let mut clock = PacketClock::default();
            assert_eq!(
                clock
                    .resolve(0, 1_000_000, 480, 0, SAMPLE_RATE, 1_100_000)
                    .unwrap(),
                1_000_000
            );
            let error = AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32;
            assert_eq!(
                clock
                    .resolve(480, 0, 480, error, SAMPLE_RATE, 1_200_000)
                    .unwrap(),
                1_100_000
            );
            assert_eq!(
                clock
                    .resolve(960, 0, 480, error, SAMPLE_RATE, 1_300_000)
                    .unwrap(),
                1_200_000
            );
            assert!(clock
                .resolve(1440, 0, 480, error, SAMPLE_RATE, 1_400_000)
                .is_err());
            assert!(PacketClock::default()
                .resolve(0, 0, 480, error, SAMPLE_RATE, 1_000_000)
                .is_err());
        }
        #[test]
        fn armed_queue_overflow_is_not_silent_drop() {
            let mut queue = Queue::default();
            for n in 0..200 {
                queue.push(Packet {
                    qpc: n * 100_000,
                    frames: 480,
                    discontinuity: false,
                    bytes: vec![0; 1920],
                });
            }
            assert!(queue.failure.is_none());
            assert!(queue.packets.len() < MAX_RAW_PACKETS);
            queue.armed = true;
            queue.push(Packet {
                qpc: 100_000_000,
                frames: 480,
                discontinuity: false,
                bytes: vec![0; 1920],
            });
            assert_eq!(queue.failure, Some(RecordingStopReason::AudioOverrun));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_are_strict_strings_and_default_is_mute() {
        assert_eq!(AudioMode::default(), AudioMode::Mute);
        for mode in ["mute", "system", "microphone", "both"] {
            assert!(serde_json::from_value::<AudioMode>(serde_json::json!(mode)).is_ok());
        }
        assert!(serde_json::from_str::<AudioMode>(r#"{"system":null}"#).is_err());
        assert!(serde_json::from_str::<AudioMode>(r#""System""#).is_err());
        assert!(serde_json::from_str::<RecordingStopReason>(r#"{"audio_overrun":null}"#).is_err());
    }
}
