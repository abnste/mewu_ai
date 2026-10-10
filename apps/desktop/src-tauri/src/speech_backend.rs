// SPDX-License-Identifier: MPL-2.0
//! Local SAPI adapter. COM objects never leave the worker's MTA thread.
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_TEXT_CHARS: usize = 8_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "String", into = "String")]
pub enum VoiceLanguage {
    System,
    ZhCn,
    EnUs,
}
impl TryFrom<String> for VoiceLanguage {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "system" => Ok(Self::System),
            "zh-CN" => Ok(Self::ZhCn),
            "en-US" => Ok(Self::EnUs),
            _ => Err("unsupported voice language"),
        }
    }
}
impl From<VoiceLanguage> for String {
    fn from(value: VoiceLanguage) -> Self {
        match value {
            VoiceLanguage::System => "system",
            VoiceLanguage::ZhCn => "zh-CN",
            VoiceLanguage::EnUs => "en-US",
        }
        .into()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceLanguageInfo {
    pub tag: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceCapabilities {
    pub supported: bool,
    pub languages: Vec<VoiceLanguageInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VoiceConfidence {
    Recognized,
    Candidate,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DictationResult {
    pub text: String,
    pub language: String,
    pub confidence: VoiceConfidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VoicePhase {
    Starting,
    Listening,
    Stopping,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerConfig {
    Capabilities,
    Dictation { language: VoiceLanguage },
}
impl<'de> Deserialize<'de> for WorkerConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Capabilities {},
            Dictation { language: VoiceLanguage },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Capabilities {} => Self::Capabilities,
            Wire::Dictation { language } => Self::Dictation { language },
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "result",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum WorkerOutput {
    Capabilities(VoiceCapabilities),
    Dictation(DictationResult),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpeechError {
    Unsupported,
    Busy,
    Cancelled,
    LanguageUnavailable,
    RecognizerUnavailable,
    AudioUnavailable,
    NoSpeech,
    TimedOut,
    TextTooLong,
    InvalidResult,
    WorkerUnavailable,
    Protocol,
    CleanupTimedOut,
}

impl fmt::Display for SpeechError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "此平台尚不支持语音输入",
            Self::Busy => "语音输入正在结束，请稍后再试",
            Self::Cancelled => "语音输入已取消",
            Self::LanguageUnavailable => "未安装所选语言的语音识别器",
            Self::RecognizerUnavailable => "无法初始化本机语音识别器",
            Self::AudioUnavailable => "无法打开麦克风，请检查设备与权限",
            Self::NoSpeech => "未识别到语音",
            Self::TimedOut => "语音输入超时",
            Self::TextTooLong => "识别结果过长，请分段输入",
            Self::InvalidResult => "语音识别器返回了无效文字",
            Self::WorkerUnavailable => "无法启动语音输入",
            Self::Protocol => "语音输入进程返回异常",
            Self::CleanupTimedOut => "语音输入尚未结束，请稍后再试",
        })
    }
}
impl std::error::Error for SpeechError {}

pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), SpeechError> {
    if cancel.load(Ordering::Acquire) {
        Err(SpeechError::Cancelled)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_text(text: &str) -> Result<(), SpeechError> {
    if text.len() > MAX_TEXT_BYTES || text.chars().count() > MAX_TEXT_CHARS {
        return Err(SpeechError::TextTooLong);
    }
    if text.trim().is_empty()
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
    {
        return Err(SpeechError::InvalidResult);
    }
    Ok(())
}

fn supported_tag(tag: &str) -> Option<&'static str> {
    if tag.eq_ignore_ascii_case("zh-CN") {
        Some("zh-CN")
    } else if tag.eq_ignore_ascii_case("en-US") {
        Some("en-US")
    } else {
        None
    }
}

fn select_language<'a>(
    requested: VoiceLanguage,
    installed: &'a [String],
    system: &[String],
) -> Option<&'a str> {
    let preferences = match requested {
        VoiceLanguage::System => system.to_vec(),
        VoiceLanguage::ZhCn => vec!["zh-CN".into()],
        VoiceLanguage::EnUs => vec!["en-US".into()],
    };
    for wanted in preferences {
        if let Some(exact) = installed.iter().find(|v| v.eq_ignore_ascii_case(&wanted)) {
            return Some(exact);
        }
        // Match the original language-family fallback, never an unrelated first engine.
        let lower = wanted.to_ascii_lowercase();
        let family = if lower == "en" || lower.starts_with("en-") {
            Some("en-US")
        } else if matches!(lower.as_str(), "zh" | "zh-hans" | "zh-cn" | "zh-sg") {
            Some("zh-CN")
        } else {
            None
        };
        if let Some(tag) = family {
            if let Some(found) = installed.iter().find(|v| v.eq_ignore_ascii_case(tag)) {
                return Some(found);
            }
        }
    }
    None
}

pub(crate) fn validate_output(
    config: WorkerConfig,
    output: &WorkerOutput,
) -> Result<(), SpeechError> {
    match (config, output) {
        (WorkerConfig::Capabilities, WorkerOutput::Capabilities(cap)) => {
            let mut tags = std::collections::HashSet::new();
            if cap.languages.len() > 2
                || cap.supported != !cap.languages.is_empty()
                || cap
                    .unavailable_reason
                    .as_ref()
                    .is_some_and(|v| v.len() > 512 || v.chars().any(char::is_control))
                || cap.languages.iter().any(|v| {
                    supported_tag(&v.tag) != Some(v.tag.as_str())
                        || !tags.insert(&v.tag)
                        || v.name.is_empty()
                        || v.name.len() > 256
                        || v.name.chars().any(char::is_control)
                })
            {
                return Err(SpeechError::Protocol);
            }
            Ok(())
        }
        (WorkerConfig::Dictation { language }, WorkerOutput::Dictation(result)) => {
            let valid_language = match language {
                VoiceLanguage::System => supported_tag(&result.language).is_some(),
                VoiceLanguage::ZhCn => result.language == "zh-CN",
                VoiceLanguage::EnUs => result.language == "en-US",
            };
            if !valid_language {
                return Err(SpeechError::Protocol);
            }
            validate_text(&result.text)
        }
        _ => Err(SpeechError::Protocol),
    }
}

pub(crate) fn execute(
    config: WorkerConfig,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(VoicePhase),
) -> Result<WorkerOutput, SpeechError> {
    check_cancel(cancel)?;
    #[cfg(windows)]
    {
        platform::execute(config, cancel, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = (config, progress);
        Err(SpeechError::Unsupported)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        ffi::c_void,
        ptr,
        time::{Duration, Instant},
    };
    use windows::{
        core::{w, IUnknown, Interface, PCWSTR, PWSTR},
        Win32::{Media::Speech::*, System::Com::*},
    };

    struct Apartment;
    impl Apartment {
        fn enter() -> Result<Self, SpeechError> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED)
                    .ok()
                    .map_err(|_| SpeechError::RecognizerUnavailable)?;
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

    struct ComString(PWSTR);
    impl ComString {
        fn read(&self, max_units: usize) -> Result<String, SpeechError> {
            if self.0.is_null() {
                return Err(SpeechError::InvalidResult);
            }
            // All pointers here are SAPI-owned, nul-terminated CoTaskMem strings.
            let mut len = 0;
            unsafe {
                while len <= max_units {
                    if *self.0 .0.add(len) == 0 {
                        return String::from_utf16(std::slice::from_raw_parts(self.0 .0, len))
                            .map_err(|_| SpeechError::InvalidResult);
                    }
                    len += 1;
                }
            }
            Err(SpeechError::TextTooLong)
        }
    }
    impl Drop for ComString {
        fn drop(&mut self) {
            unsafe {
                CoTaskMemFree(Some(self.0 .0.cast()));
            }
        }
    }

    struct EngineToken {
        token: ISpObjectToken,
        languages: Vec<String>,
    }
    fn category(id: PCWSTR) -> Result<ISpObjectTokenCategory, SpeechError> {
        unsafe {
            let category: ISpObjectTokenCategory =
                CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_INPROC_SERVER)
                    .map_err(|_| SpeechError::RecognizerUnavailable)?;
            category
                .SetId(id, false)
                .map_err(|_| SpeechError::RecognizerUnavailable)?;
            Ok(category)
        }
    }
    fn locale_name(id: u32) -> Option<String> {
        let mut name = [0u16; 85];
        let len = unsafe {
            windows_sys::Win32::Globalization::LCIDToLocaleName(
                id,
                name.as_mut_ptr(),
                name.len() as i32,
                0,
            )
        };
        (len > 1).then(|| String::from_utf16_lossy(&name[..len as usize - 1]))
    }
    fn system_languages() -> Vec<String> {
        use windows_sys::Win32::Globalization::{GetUserDefaultLCID, GetUserDefaultUILanguage};
        unsafe {
            [
                locale_name(GetUserDefaultUILanguage() as u32),
                locale_name(GetUserDefaultLCID()),
            ]
            .into_iter()
            .flatten()
            .collect()
        }
    }
    fn enumerate(cancel: &AtomicBool) -> Result<Vec<EngineToken>, SpeechError> {
        check_cancel(cancel)?;
        let category = category(SPCAT_RECOGNIZERS)?;
        let list = unsafe { category.EnumTokens(PCWSTR::null(), PCWSTR::null()) }
            .map_err(|_| SpeechError::RecognizerUnavailable)?;
        let mut count = 0;
        unsafe { list.GetCount(&mut count) }.map_err(|_| SpeechError::RecognizerUnavailable)?;
        if count > 64 {
            return Err(SpeechError::RecognizerUnavailable);
        }
        let mut result = Vec::new();
        for index in 0..count {
            check_cancel(cancel)?;
            let token =
                unsafe { list.Item(index) }.map_err(|_| SpeechError::RecognizerUnavailable)?;
            let attrs = match unsafe { token.OpenKey(w!("Attributes")) } {
                Ok(v) => v,
                Err(_) => continue,
            };
            let languages = match unsafe { attrs.GetStringValue(w!("Language")) } {
                Ok(v) => ComString(v).read(256)?,
                Err(_) => continue,
            };
            let mut supported = Vec::new();
            for field in languages.split(';').take(16) {
                let field = field.trim();
                if field.is_empty()
                    || field.len() > 8
                    || !field.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    continue;
                }
                if let Some(tag) = u32::from_str_radix(field, 16)
                    .ok()
                    .and_then(locale_name)
                    .as_deref()
                    .and_then(supported_tag)
                {
                    if !supported.iter().any(|v| v == tag) {
                        supported.push(tag.to_string());
                    }
                }
            }
            if !supported.is_empty() {
                result.push(EngineToken {
                    token,
                    languages: supported,
                });
            }
        }
        Ok(result)
    }
    fn installed(tokens: &[EngineToken]) -> Vec<String> {
        let mut languages = tokens
            .iter()
            .flat_map(|e| e.languages.clone())
            .collect::<Vec<_>>();
        languages.sort();
        languages.dedup();
        languages
    }
    fn capabilities(tokens: &[EngineToken]) -> VoiceCapabilities {
        let languages = installed(tokens)
            .into_iter()
            .map(|tag| VoiceLanguageInfo {
                name: if tag == "zh-CN" {
                    "简体中文"
                } else {
                    "English (US)"
                }
                .into(),
                tag,
            })
            .collect::<Vec<_>>();
        VoiceCapabilities {
            supported: !languages.is_empty(),
            unavailable_reason: languages
                .is_empty()
                .then(|| "未安装支持的本机语音识别器".into()),
            languages,
        }
    }

    // SPEVENT owns an interface reference or a CoTaskMem allocation, as specified
    // by the Windows SDK's SpClearEvent contract. The queue's event handle is borrowed.
    struct Event(SPEVENT);
    impl Event {
        fn id(&self) -> i32 {
            (self.0._bitfield as u32 & 0xffff) as i32
        }
        fn payload_type(&self) -> i32 {
            ((self.0._bitfield as u32 >> 16) & 0xffff) as i32
        }
        fn text(&self) -> Result<String, SpeechError> {
            if self.payload_type() != SPET_LPARAM_IS_OBJECT.0 || self.0.lParam.0 == 0 {
                return Err(SpeechError::InvalidResult);
            }
            let raw = self.0.lParam.0 as *mut c_void;
            unsafe {
                let object = IUnknown::from_raw_borrowed(&raw).ok_or(SpeechError::InvalidResult)?;
                let result: ISpRecoResult =
                    object.cast().map_err(|_| SpeechError::InvalidResult)?;
                let mut text = ComString(PWSTR::null());
                result
                    .GetText(0, u32::MAX, true, &mut text.0, None)
                    .map_err(|_| SpeechError::InvalidResult)?;
                let text = text.read(MAX_TEXT_BYTES)?.trim().to_string();
                validate_text(&text)?;
                Ok(text)
            }
        }
    }
    impl Drop for Event {
        fn drop(&mut self) {
            if self.0.lParam.0 == 0 {
                return;
            }
            unsafe {
                let raw = self.0.lParam.0 as *mut c_void;
                match self.payload_type() {
                    1 | 2 => drop(IUnknown::from_raw(raw)),
                    3 | 4 => CoTaskMemFree(Some(raw)),
                    _ => {}
                }
            }
        }
    }
    fn events(context: &ISpRecoContext) -> Result<Vec<Event>, SpeechError> {
        let mut raw = [SPEVENT::default(); 32];
        let mut count = 0;
        let status = unsafe { context.GetEvents(raw.len() as u32, raw.as_mut_ptr(), &mut count) };
        let owned = raw
            .into_iter()
            .take((count as usize).min(32))
            .map(Event)
            .collect();
        status.map_err(|_| SpeechError::RecognizerUnavailable)?;
        if count > 32 {
            return Err(SpeechError::InvalidResult);
        }
        Ok(owned)
    }
    struct RecoStop(ISpRecognizer);
    impl Drop for RecoStop {
        fn drop(&mut self) {
            unsafe {
                let _ = self.0.SetRecoState(SPRST_INACTIVE_WITH_PURGE);
            }
        }
    }
    struct GrammarStop(ISpRecoGrammar);
    impl Drop for GrammarStop {
        fn drop(&mut self) {
            unsafe {
                let _ = self.0.SetDictationState(SPRS_INACTIVE);
            }
        }
    }

    fn dictate(
        tokens: &[EngineToken],
        requested: VoiceLanguage,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(VoicePhase),
    ) -> Result<DictationResult, SpeechError> {
        let tags = installed(tokens);
        let language = select_language(requested, &tags, &system_languages())
            .ok_or(SpeechError::LanguageUnavailable)?
            .to_string();
        let token = tokens
            .iter()
            .find(|e| e.languages.contains(&language))
            .ok_or(SpeechError::LanguageUnavailable)?;
        check_cancel(cancel)?;
        let recognizer: ISpRecognizer =
            unsafe { CoCreateInstance(&SpInprocRecognizer, None, CLSCTX_INPROC_SERVER) }
                .map_err(|_| SpeechError::RecognizerUnavailable)?;
        let recognizer = RecoStop(recognizer);
        unsafe {
            recognizer
                .0
                .SetRecognizer(&token.token)
                .map_err(|_| SpeechError::RecognizerUnavailable)?;
            recognizer
                .0
                .SetRecoState(SPRST_INACTIVE)
                .map_err(|_| SpeechError::RecognizerUnavailable)?;
        }
        let context = unsafe { recognizer.0.CreateRecoContext() }
            .map_err(|_| SpeechError::RecognizerUnavailable)?;
        let grammar = GrammarStop(
            unsafe { context.CreateGrammar(1) }.map_err(|_| SpeechError::RecognizerUnavailable)?,
        );
        unsafe {
            context
                .SetAudioOptions(SPAO_NONE, ptr::null(), ptr::null())
                .map_err(|_| SpeechError::RecognizerUnavailable)?;
            context
                .SetNotifyWin32Event()
                .map_err(|_| SpeechError::RecognizerUnavailable)?;
            let flags = (1u64 << SPEI_RESERVED1.0) | (1u64 << SPEI_RESERVED2.0);
            let interest = [
                SPEI_RECOGNITION,
                SPEI_FALSE_RECOGNITION,
                SPEI_HYPOTHESIS,
                SPEI_SOUND_START,
                SPEI_PHRASE_START,
                SPEI_END_SR_STREAM,
            ]
            .iter()
            .fold(flags, |bits, e| bits | (1u64 << e.0));
            context
                .SetInterest(interest, interest)
                .map_err(|_| SpeechError::RecognizerUnavailable)?;
            grammar
                .0
                .LoadDictation(PCWSTR::null(), SPLO_STATIC)
                .map_err(|_| SpeechError::RecognizerUnavailable)?;
        }
        let audio = category(SPCAT_AUDIOIN).map_err(|_| SpeechError::AudioUnavailable)?;
        let audio_id = ComString(
            unsafe { audio.GetDefaultTokenId() }.map_err(|_| SpeechError::AudioUnavailable)?,
        );
        audio_id
            .read(4096)
            .map_err(|_| SpeechError::AudioUnavailable)?;
        let input: ISpObjectToken =
            unsafe { CoCreateInstance(&SpObjectToken, None, CLSCTX_INPROC_SERVER) }
                .map_err(|_| SpeechError::AudioUnavailable)?;
        unsafe { input.SetId(SPCAT_AUDIOIN, PCWSTR(audio_id.0 .0), false) }
            .map_err(|_| SpeechError::AudioUnavailable)?;
        check_cancel(cancel)?;
        // Only this explicit dictation branch opens input. Capability enumeration
        // never creates a recognizer, input token or active grammar.
        unsafe { recognizer.0.SetInput(&input, true) }
            .map_err(|_| SpeechError::AudioUnavailable)?;
        check_cancel(cancel)?;
        unsafe {
            grammar
                .0
                .SetDictationState(SPRS_ACTIVE_WITH_AUTO_PAUSE)
                .map_err(|_| SpeechError::RecognizerUnavailable)?;
            recognizer
                .0
                .SetRecoState(SPRST_ACTIVE)
                .map_err(|_| SpeechError::AudioUnavailable)?;
        }
        check_cancel(cancel)?;
        progress(VoicePhase::Listening);
        let result = listen(&context, language, cancel);
        progress(VoicePhase::Stopping);
        // Cleanup completes before the worker emits its final message. If a driver
        // stalls in any COM call, the supervising process kills only this worker job.
        result
    }
    fn listen(
        context: &ISpRecoContext,
        language: String,
        cancel: &AtomicBool,
    ) -> Result<DictationResult, SpeechError> {
        let started = Instant::now();
        let mut heard = false;
        let mut candidate = None;
        loop {
            check_cancel(cancel)?;
            if started.elapsed() >= Duration::from_secs(60) {
                return Err(SpeechError::TimedOut);
            }
            if !heard && started.elapsed() >= Duration::from_secs(8) {
                return Err(SpeechError::NoSpeech);
            }
            for event in events(context)? {
                check_cancel(cancel)?;
                match event.id() {
                    id if id == SPEI_SOUND_START.0 || id == SPEI_PHRASE_START.0 => heard = true,
                    id if id == SPEI_RECOGNITION.0 => {
                        return Ok(DictationResult {
                            text: event.text()?,
                            language,
                            confidence: VoiceConfidence::Recognized,
                        })
                    }
                    id if id == SPEI_HYPOTHESIS.0 => {
                        heard = true;
                        match event.text() {
                            Ok(text) => candidate = Some(text),
                            Err(SpeechError::TextTooLong) => return Err(SpeechError::TextTooLong),
                            Err(_) => {}
                        }
                    }
                    id if id == SPEI_FALSE_RECOGNITION.0 => {
                        let text = match event.text() {
                            Ok(text) => text,
                            Err(SpeechError::TextTooLong) => return Err(SpeechError::TextTooLong),
                            Err(_) => candidate.ok_or(SpeechError::NoSpeech)?,
                        };
                        return Ok(DictationResult {
                            text,
                            language,
                            confidence: VoiceConfidence::Candidate,
                        });
                    }
                    id if id == SPEI_END_SR_STREAM.0 => {
                        if (event.0.lParam.0 as i32) < 0 {
                            return Err(SpeechError::AudioUnavailable);
                        }
                        return candidate
                            .map(|text| DictationResult {
                                text,
                                language,
                                confidence: VoiceConfidence::Candidate,
                            })
                            .ok_or(SpeechError::NoSpeech);
                    }
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    pub(super) fn execute(
        config: WorkerConfig,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(VoicePhase),
    ) -> Result<WorkerOutput, SpeechError> {
        let _apartment = Apartment::enter()?;
        let tokens = enumerate(cancel)?;
        check_cancel(cancel)?;
        let output = match config {
            WorkerConfig::Capabilities => WorkerOutput::Capabilities(capabilities(&tokens)),
            WorkerConfig::Dictation { language } => {
                WorkerOutput::Dictation(dictate(&tokens, language, cancel, progress)?)
            }
        };
        check_cancel(cancel)?;
        validate_output(config, &output)?;
        Ok(output)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::{
            core::GUID,
            Win32::{
                Foundation::HGLOBAL, Media::Audio::WAVEFORMATEX,
                System::Com::StructuredStorage::CreateStreamOnHGlobal,
            },
        };

        #[test]
        #[ignore = "Explicit local SAPI test: owned subprocess, synthetic memory PCM/text, no microphone or audio files"]
        fn sapi_emulation_uses_only_memory_audio() {
            const TEST: &str =
                "speech_backend::platform::tests::sapi_emulation_uses_only_memory_audio";
            if std::env::var("MEWU_SPEECH_SAPI_TEST_CHILD").as_deref() != Ok("1") {
                crate::speech_worker::run_isolated_platform_test(TEST).unwrap();
                return;
            }
            let mut gate = String::new();
            std::io::stdin().read_line(&mut gate).unwrap();
            assert_eq!(gate, "go\n", "parent assigned owned Job before COM");
            let apartment = Apartment::enter().unwrap();
            let tokens = enumerate(&AtomicBool::new(false)).unwrap();
            let token = tokens
                .iter()
                .find(|token| token.languages.iter().any(|tag| tag == "zh-CN"))
                .expect("explicit platform test requires an installed zh-CN SAPI recognizer");
            let recognizer: ISpRecognizer =
                unsafe { CoCreateInstance(&SpInprocRecognizer, None, CLSCTX_INPROC_SERVER) }
                    .unwrap();
            let recognizer = RecoStop(recognizer);
            unsafe {
                recognizer.0.SetRecognizer(&token.token).unwrap();
                recognizer.0.SetRecoState(SPRST_INACTIVE).unwrap();
            }
            // The only input is zeroed PCM held in memory. No SPCAT_AUDIOIN or
            // default-input lookup occurs, and nothing writes this source after
            // setup. EmulateRecognition supplies text, not recorded speech.
            let memory = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) }.unwrap();
            let silence = vec![0u8; 16_000 * 2 * 60];
            let mut written = 0;
            unsafe {
                memory
                    .Write(
                        silence.as_ptr().cast(),
                        silence.len() as u32,
                        Some(&mut written),
                    )
                    .ok()
                    .unwrap();
                assert_eq!(written as usize, silence.len());
                memory.Seek(0, STREAM_SEEK_SET, None).unwrap();
            }
            let stream: ISpStream =
                unsafe { CoCreateInstance(&SpStream, None, CLSCTX_INPROC_SERVER) }.unwrap();
            // The SAPI format GUID is defined in the Microsoft Windows SDK and
            // System.Speech's SapiInterop definitions (SPDFID_WaveFormatEx).
            let format_id = GUID::from_u128(0xc31adbae_527f_4ff5_a230_f62bb61ff70c);
            let format = WAVEFORMATEX {
                wFormatTag: 1,
                nChannels: 1,
                nSamplesPerSec: 16_000,
                nAvgBytesPerSec: 32_000,
                nBlockAlign: 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            unsafe {
                stream.SetBaseStream(&memory, &format_id, &format).unwrap();
                recognizer.0.SetInput(&stream, true).unwrap();
            }
            let context = unsafe { recognizer.0.CreateRecoContext() }.unwrap();
            let grammar = GrammarStop(unsafe { context.CreateGrammar(1) }.unwrap());
            unsafe {
                context
                    .SetAudioOptions(SPAO_NONE, ptr::null(), ptr::null())
                    .unwrap();
                context.SetNotifyWin32Event().unwrap();
                let interest = (1u64 << SPEI_RESERVED1.0)
                    | (1u64 << SPEI_RESERVED2.0)
                    | (1u64 << SPEI_RECOGNITION.0)
                    | (1u64 << SPEI_FALSE_RECOGNITION.0);
                context.SetInterest(interest, interest).unwrap();
                grammar
                    .0
                    .LoadDictation(PCWSTR::null(), SPLO_STATIC)
                    .unwrap();
                grammar
                    .0
                    .SetDictationState(SPRS_ACTIVE_WITH_AUTO_PAUSE)
                    .unwrap();
                recognizer.0.SetRecoState(SPRST_ACTIVE_ALWAYS).unwrap();
            }
            let words = [w!("你好"), w!("世界")];
            let elements = words.map(|word| SPPHRASEELEMENT {
                pszDisplayText: word,
                pszLexicalForm: word,
                bDisplayAttributes: SPAF_ONE_TRAILING_SPACE.0 as u8,
                ActualConfidence: SP_NORMAL_CONFIDENCE as i8,
                RequiredConfidence: SP_NORMAL_CONFIDENCE as i8,
                SREngineConfidence: 1.0,
                ..Default::default()
            });
            let mut phrase = SPPHRASE::default();
            phrase.Base.cbSize = std::mem::size_of::<SPPHRASE>() as u32;
            phrase.Base.LangID = 0x0804;
            phrase.Base.Rule.ulCountOfElements = elements.len() as u32;
            phrase.Base.Rule.SREngineConfidence = 1.0;
            phrase.Base.pElements = elements.as_ptr();
            let builder: ISpPhraseBuilder =
                unsafe { CoCreateInstance(&SpPhraseBuilder, None, CLSCTX_INPROC_SERVER) }.unwrap();
            unsafe {
                builder.InitFromPhrase(&phrase).unwrap();
                recognizer.0.EmulateRecognition(&builder).unwrap();
            }
            let started = Instant::now();
            let mut found = None;
            while found.is_none() && started.elapsed() < Duration::from_secs(5) {
                for event in events(&context).unwrap() {
                    if event.id() == SPEI_RECOGNITION.0 {
                        found = Some(event.text().unwrap());
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let text =
                found.expect("emulated recognition arrived through the production event decoder");
            assert_eq!(
                text.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>(),
                "你好世界"
            );
            // Explicitly complete the same RAII release path before success can
            // leave this process; the parent additionally verifies Job emptiness.
            drop(builder);
            drop(grammar);
            drop(context);
            drop(recognizer);
            drop(stream);
            drop(memory);
            drop(tokens);
            drop(apartment);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_selection_never_uses_unrelated_engine() {
        let installed = vec!["zh-CN".into()];
        assert_eq!(
            select_language(VoiceLanguage::System, &installed, &["zh-SG".into()]),
            Some("zh-CN")
        );
        assert_eq!(
            select_language(VoiceLanguage::System, &installed, &["zh-TW".into()]),
            None
        );
        assert_eq!(
            select_language(VoiceLanguage::EnUs, &installed, &["zh-CN".into()]),
            None
        );
        assert_eq!(
            select_language(VoiceLanguage::System, &installed, &["de-DE".into()]),
            None
        );
    }
    #[test]
    fn text_bounds_reject_without_truncating() {
        assert!(validate_text(&"中".repeat(8000)).is_ok());
        assert_eq!(
            validate_text(&"中".repeat(8001)),
            Err(SpeechError::TextTooLong)
        );
        assert_eq!(validate_text("\0hello"), Err(SpeechError::InvalidResult));
        assert_eq!(validate_text("  \n"), Err(SpeechError::InvalidResult));
        assert!(validate_text("一行\n第二行\ttext").is_ok());
    }
    #[test]
    fn worker_result_must_match_requested_mode_and_language() {
        let result = WorkerOutput::Dictation(DictationResult {
            text: "测试".into(),
            language: "zh-CN".into(),
            confidence: VoiceConfidence::Recognized,
        });
        assert_eq!(
            validate_output(WorkerConfig::Capabilities, &result),
            Err(SpeechError::Protocol)
        );
        assert_eq!(
            validate_output(
                WorkerConfig::Dictation {
                    language: VoiceLanguage::EnUs
                },
                &result
            ),
            Err(SpeechError::Protocol)
        );
        assert!(
            serde_json::from_str::<WorkerConfig>(r#"{"mode":"dictation","language":"fr-FR"}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<WorkerConfig>(
            r#"{"mode":"capabilities","microphone":true}"#
        )
        .is_err());
        assert!(serde_json::from_str::<VoiceLanguage>(r#"{"zh-CN":null}"#).is_err());
    }
    #[test]
    fn cancelled_backend_never_enters_com() {
        let cancel = AtomicBool::new(true);
        assert_eq!(
            execute(WorkerConfig::Capabilities, &cancel, &mut |_| panic!(
                "progress"
            )),
            Err(SpeechError::Cancelled)
        );
    }
}
