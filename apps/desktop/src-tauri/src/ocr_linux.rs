// SPDX-License-Identifier: MPL-2.0
//! Blocking Tesseract 4.1/5 C API, never a CLI or executable plugin. Native handles,
//! owned pixels and the caller's worker lease survive cooperative cancellation
//! until the synchronous call returns. Init3 itself is not cancellable.
//! https://github.com/tesseract-ocr/tesseract/blob/5.5.1/include/tesseract/capi.h
//! https://github.com/tesseract-ocr/tesseract/blob/5.5.1/src/api/capi.cpp
//! https://github.com/tesseract-ocr/tesseract/blob/4.1.1/src/api/capi.h
//! https://github.com/tesseract-ocr/tesseract/blob/4.1.1/src/api/capi.cpp
use super::*;
use libloading::Library;
use mewu_core::{validate_ocr_document, OcrLine, OcrWord};
use std::{
    ffi::{c_char, c_int, c_void, CString},
    fs::{self, File},
    os::unix::{fs::MetadataExt, io::AsRawFd},
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::atomic::AtomicUsize,
};
use zeroize::Zeroizing;

const UNAVAILABLE: &str =
    "本机文字识别需要受保护的 Tesseract 4.1 或 5 和英语语言包，请安装对应系统软件包后重试";
const NATIVE_ERROR: &str = "本机文字识别失败，请检查系统文字识别组件";
const RESULT_ERROR: &str = "本机文字识别结果无效或超过上限";
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_LINES: usize = 512;
const MAX_WORDS: usize = 4096;
const MAX_TEXT: usize = 128 * 1024;
static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
thread_local! {
    static NATIVE_TEST_TRACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
// Only the explicit synthetic native fixture enables these content-free
// markers. Release builds contain neither the flag nor diagnostic output.
macro_rules! native_trace {
    ($stage:literal) => {
        #[cfg(test)]
        if NATIVE_TEST_TRACE.with(std::cell::Cell::get) {
            eprintln!("MEWU_LINUX_OCR_STAGE {}", $stage);
        }
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Abi {
    Five,
    FourOne,
}
impl Abi {
    fn soname(self) -> &'static str {
        match self {
            Self::Five => "libtesseract.so.5",
            Self::FourOne => "libtesseract.so.4",
        }
    }
    fn model_roots(self) -> [&'static str; 2] {
        [
            match self {
                Self::Five => "/usr/share/tesseract-ocr/5/tessdata",
                Self::FourOne => "/usr/share/tesseract-ocr/4.00/tessdata",
            },
            "/usr/share/tessdata",
        ]
    }
    fn accepts_version(self, version: &str) -> bool {
        version.starts_with(match self {
            Self::Five => "5.",
            Self::FourOne => "4.1.",
        })
    }
}

// Only an absent preferred SONAME permits fallback. Permission errors,
// dangling symlinks and unsafe existing libraries must fail closed.
fn select_abi(mut present: impl FnMut(&str) -> Result<bool, String>) -> Result<Abi, String> {
    for abi in [Abi::Five, Abi::FourOne] {
        if present(abi.soname())? {
            return Ok(abi);
        }
    }
    Err(UNAVAILABLE.into())
}
fn path_present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(UNAVAILABLE.into()),
    }
}

struct Permit;
impl Permit {
    fn acquire() -> Result<Self, String> {
        ACTIVE_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            })
            .map_err(|_| "已有文字识别正在进行，请稍后重试".to_owned())?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

// Root ownership is deliberately strict. UID 65534 in a user namespace is
// not accepted as a substitute for verified host-root ownership. A future
// isolated Code caller needs a separate sealed outer-namespace attestation.
fn protected_path(path: &Path, base: &Path, directory: bool) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path).map_err(|_| UNAVAILABLE.to_owned())?;
    let base = fs::canonicalize(base).map_err(|_| UNAVAILABLE.to_owned())?;
    if !canonical.starts_with(&base) {
        return Err(UNAVAILABLE.into());
    }
    // Both the original route and resolved route must be protected: checking
    // only the final target would admit a writable symlink parent.
    for route in [path, canonical.as_path()] {
        for ancestor in route.ancestors() {
            let metadata = fs::symlink_metadata(ancestor).map_err(|_| UNAVAILABLE.to_owned())?;
            if metadata.uid() != 0
                || (!metadata.file_type().is_symlink() && metadata.mode() & 0o022 != 0)
                || (!metadata.file_type().is_symlink() && ancestor != route && !metadata.is_dir())
            {
                return Err(UNAVAILABLE.into());
            }
        }
    }
    let metadata = fs::metadata(&canonical).map_err(|_| UNAVAILABLE.to_owned())?;
    if (directory && !metadata.is_dir())
        || (!directory && (!metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_FILE))
    {
        return Err(UNAVAILABLE.into());
    }
    Ok(canonical)
}

struct Resources {
    abi: Abi,
    library: File,
    models: Vec<(PathBuf, File)>,
    data: CString,
    language: CString,
}
impl Resources {
    fn capture() -> Result<Self, String> {
        #[cfg(target_arch = "x86_64")]
        let library_root = Path::new("/usr/lib/x86_64-linux-gnu");
        #[cfg(target_arch = "aarch64")]
        let library_root = Path::new("/usr/lib/aarch64-linux-gnu");
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        return Err(UNAVAILABLE.into());
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        {
            let abi = select_abi(|soname| path_present(&library_root.join(soname)))?;
            let library_path =
                protected_path(&library_root.join(abi.soname()), library_root, false)?;
            let library = File::open(library_path).map_err(|_| UNAVAILABLE.to_owned())?;
            let mut chosen = None;
            for root in abi.model_roots() {
                let root = Path::new(root);
                if !path_present(root)? {
                    continue;
                }
                let data = protected_path(root, root, true)?;
                let english = data.join("eng.traineddata");
                if path_present(&english)? {
                    protected_path(&english, &data, false)?;
                    chosen = Some(data);
                    break;
                }
            }
            let data = chosen.ok_or_else(|| UNAVAILABLE.to_owned())?;
            let mut models = Vec::new();
            let mut languages = Vec::new();
            for language in ["eng", "chi_sim", "chi_tra"] {
                let path = data.join(format!("{language}.traineddata"));
                match protected_path(&path, &data, false) {
                    Ok(path) => {
                        let file = File::open(&path).map_err(|_| UNAVAILABLE.to_owned())?;
                        let metadata = file.metadata().map_err(|_| UNAVAILABLE.to_owned())?;
                        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
                            return Err(UNAVAILABLE.into());
                        }
                        models.push((path, file));
                        languages.push(language);
                    }
                    // Absence is optional; an existing unsafe Chinese model
                    // must not be silently represented as unavailable.
                    Err(_)
                        if language != "eng"
                            && fs::symlink_metadata(&path).is_err_and(|error| {
                                error.kind() == std::io::ErrorKind::NotFound
                            }) => {}
                    Err(error) => return Err(error),
                }
            }
            let data = CString::new(data.as_os_str().as_encoded_bytes())
                .map_err(|_| UNAVAILABLE.to_owned())?;
            let language = CString::new(languages.join("+")).map_err(|_| UNAVAILABLE.to_owned())?;
            let resources = Self {
                abi,
                library,
                models,
                data,
                language,
            };
            resources.recheck()?;
            Ok(resources)
        }
    }
    fn recheck(&self) -> Result<(), String> {
        let library = self
            .library
            .metadata()
            .map_err(|_| UNAVAILABLE.to_owned())?;
        if library.uid() != 0 || library.mode() & 0o022 != 0 || !library.is_file() {
            return Err(UNAVAILABLE.into());
        }
        for (path, file) in &self.models {
            let held = file.metadata().map_err(|_| UNAVAILABLE.to_owned())?;
            let current = fs::metadata(path).map_err(|_| UNAVAILABLE.to_owned())?;
            if held.uid() != 0
                || held.mode() & 0o022 != 0
                || (
                    held.dev(),
                    held.ino(),
                    held.len(),
                    held.mtime(),
                    held.mtime_nsec(),
                ) != (
                    current.dev(),
                    current.ino(),
                    current.len(),
                    current.mtime(),
                    current.mtime_nsec(),
                )
            {
                return Err(UNAVAILABLE.into());
            }
        }
        Ok(())
    }
}

type Create = unsafe extern "C" fn() -> *mut c_void;
type Delete = unsafe extern "C" fn(*mut c_void);
type Cancel = unsafe extern "C" fn(*mut c_void, c_int) -> bool;
struct Api {
    create: Create,
    delete: Delete,
    init: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    variable: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    segment: unsafe extern "C" fn(*mut c_void, c_int),
    image: unsafe extern "C" fn(*mut c_void, *const u8, c_int, c_int, c_int, c_int),
    recognize: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int,
    iterator: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    iterator_delete: Delete,
    page: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    beginning: unsafe extern "C" fn(*const c_void, c_int) -> c_int,
    bounding: unsafe extern "C" fn(
        *const c_void,
        c_int,
        *mut c_int,
        *mut c_int,
        *mut c_int,
        *mut c_int,
    ) -> c_int,
    text: unsafe extern "C" fn(*const c_void, c_int) -> *mut c_char,
    text_delete: unsafe extern "C" fn(*const c_char),
    next: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    monitor_create: Create,
    monitor_delete: Delete,
    cancel_func: unsafe extern "C" fn(*mut c_void, Cancel),
    cancel_this: unsafe extern "C" fn(*mut c_void, *mut c_void),
    deadline: unsafe extern "C" fn(*mut c_void, c_int),
    _library: Library,
}
impl Api {
    fn load(file: &File, abi: Abi) -> Result<Self, String> {
        // Bind dlopen to the held inode, without choosing a loader search path
        // from caller settings, environment variables or a plugin manifest.
        let library: Library = unsafe {
            libloading::os::unix::Library::open(
                Some(format!("/proc/self/fd/{}", file.as_raw_fd())),
                libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_LOCAL,
            )
        }
        .map_err(|_| UNAVAILABLE.to_owned())?
        .into();
        macro_rules! symbol {
            ($name:literal) => {
                unsafe {
                    *library
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|_| UNAVAILABLE.to_owned())?
                }
            };
        }
        let version: unsafe extern "C" fn() -> *const c_char = symbol!("TessVersion");
        if !abi.accepts_version(&unsafe { bounded_text(version(), 128)? }) {
            return Err(UNAVAILABLE.into());
        }
        Ok(Self {
            create: symbol!("TessBaseAPICreate"),
            delete: symbol!("TessBaseAPIDelete"),
            init: symbol!("TessBaseAPIInit3"),
            variable: symbol!("TessBaseAPISetVariable"),
            segment: symbol!("TessBaseAPISetPageSegMode"),
            image: symbol!("TessBaseAPISetImage"),
            recognize: symbol!("TessBaseAPIRecognize"),
            iterator: symbol!("TessBaseAPIGetIterator"),
            iterator_delete: symbol!("TessResultIteratorDelete"),
            page: symbol!("TessResultIteratorGetPageIterator"),
            beginning: symbol!("TessPageIteratorIsAtBeginningOf"),
            bounding: symbol!("TessPageIteratorBoundingBox"),
            text: symbol!("TessResultIteratorGetUTF8Text"),
            text_delete: symbol!("TessDeleteText"),
            next: symbol!("TessResultIteratorNext"),
            monitor_create: symbol!("TessMonitorCreate"),
            monitor_delete: symbol!("TessMonitorDelete"),
            cancel_func: symbol!("TessMonitorSetCancelFunc"),
            cancel_this: symbol!("TessMonitorSetCancelThis"),
            deadline: symbol!("TessMonitorSetDeadlineMSecs"),
            _library: library,
        })
    }
}

#[cfg(test)]
impl Drop for Api {
    fn drop(&mut self) {
        native_trace!("api_release_begin");
    }
}

struct Handle {
    pointer: NonNull<c_void>,
    delete: Delete,
}
impl Handle {
    fn new(pointer: *mut c_void, delete: Delete) -> Result<Self, String> {
        Ok(Self {
            pointer: NonNull::new(pointer).ok_or_else(|| NATIVE_ERROR.to_owned())?,
            delete,
        })
    }
    fn raw(&self) -> *mut c_void {
        self.pointer.as_ptr()
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        native_trace!("handle_release_begin");
        unsafe { (self.delete)(self.raw()) };
        native_trace!("handle_release_end");
    }
}
struct Text {
    pointer: *const c_char,
    delete: unsafe extern "C" fn(*const c_char),
}
impl Drop for Text {
    fn drop(&mut self) {
        unsafe { (self.delete)(self.pointer) };
    }
}
// The trusted C API owns a NUL-terminated allocation. Bound reads and Rust
// allocations; never create an unbounded CStr or log the recognized text.
unsafe fn bounded_text(pointer: *const c_char, limit: usize) -> Result<String, String> {
    if pointer.is_null() {
        return Err(RESULT_ERROR.into());
    }
    for length in 0..limit {
        if unsafe { *pointer.add(length) } == 0 {
            let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) };
            return std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|_| RESULT_ERROR.to_owned());
        }
    }
    Err(RESULT_ERROR.into())
}
struct Interest {
    cancel: Arc<AtomicBool>,
    started: Instant,
}
unsafe extern "C" fn cancelled(context: *mut c_void, _: c_int) -> bool {
    std::panic::catch_unwind(|| {
        let Some(interest) = (unsafe { context.cast::<Interest>().as_ref() }) else {
            return true;
        };
        check_interest(&interest.cancel, interest.started).is_err()
    })
    .unwrap_or(true)
}

pub(super) fn recognize_impl(
    bitmap: PixelBuffer,
    cancel: Arc<AtomicBool>,
) -> Result<OcrDocument, String> {
    let started = Instant::now();
    check_interest(&cancel, started)?;
    let _permit = Permit::acquire()?;
    let geometry = Geometry::new(bitmap.width(), bitmap.height(), MAX_SIDE)?;
    let bgra = Zeroizing::new(prepare(bitmap, geometry, &cancel, started)?);
    let mut rgb = Zeroizing::new(Vec::with_capacity(
        geometry.width as usize * geometry.height as usize * 3,
    ));
    for (index, pixel) in bgra.chunks_exact(4).enumerate() {
        if index % 16_384 == 0 {
            check_interest(&cancel, started)?;
        }
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let resources = Resources::capture();
    check_interest(&cancel, started)?;
    let resources = resources?;
    native_trace!("resources_captured");
    let api = Api::load(&resources.library, resources.abi);
    check_interest(&cancel, started)?;
    let api = api?;
    native_trace!("api_loaded");
    let engine = Handle::new(unsafe { (api.create)() }, api.delete)?;
    native_trace!("engine_created");
    if unsafe { (api.variable)(engine.raw(), c"debug_file".as_ptr(), c"/dev/null".as_ptr()) } == 0 {
        return Err(NATIVE_ERROR.into());
    }
    // No detach: Init3 may outlive cancellation; the native API and permits
    // are retained until it really returns, then the original budget is tested.
    native_trace!("init_begin");
    let initialized = unsafe {
        (api.init)(
            engine.raw(),
            resources.data.as_ptr(),
            resources.language.as_ptr(),
        )
    };
    native_trace!("init_end");
    check_interest(&cancel, started)?;
    if initialized != 0 {
        return Err(UNAVAILABLE.into());
    }
    resources.recheck()?;
    unsafe {
        (api.segment)(engine.raw(), 3); // PSM_AUTO, without OSD model access.
        (api.image)(
            engine.raw(),
            rgb.as_ptr(),
            geometry.width as c_int,
            geometry.height as c_int,
            3,
            geometry.width as c_int * 3,
        );
    }
    native_trace!("image_set");
    check_interest(&cancel, started)?;
    let mut interest = Box::new(Interest {
        cancel: cancel.clone(),
        started,
    });
    let monitor = Handle::new(unsafe { (api.monitor_create)() }, api.monitor_delete)?;
    let remaining = TIMEOUT.saturating_sub(started.elapsed()).as_millis();
    if remaining == 0 {
        return Err(TIMEOUT_ERROR.into());
    }
    unsafe {
        (api.cancel_func)(monitor.raw(), cancelled);
        (api.cancel_this)(monitor.raw(), (&mut *interest as *mut Interest).cast());
        (api.deadline)(monitor.raw(), remaining.min(c_int::MAX as u128) as c_int);
    }
    native_trace!("recognize_begin");
    let recognized = unsafe { (api.recognize)(engine.raw(), monitor.raw()) };
    native_trace!("recognize_end");
    check_interest(&cancel, started)?;
    if recognized != 0 {
        return Err(NATIVE_ERROR.into());
    }
    let mut document = OcrDocument {
        engine: "linux.tesseract".into(),
        language: resources
            .language
            .to_str()
            .map_err(|_| RESULT_ERROR.to_owned())?
            .into(),
        width: geometry.source_width,
        height: geometry.source_height,
        text_angle: None,
        lines: Vec::new(),
    };
    native_trace!("iterator_begin");
    let pointer = unsafe { (api.iterator)(engine.raw()) };
    native_trace!("iterator_end");
    if !pointer.is_null() {
        let iterator = Handle::new(pointer, api.iterator_delete)?;
        // Borrowed base-class view, never separately destroyed.
        let page = unsafe { (api.page)(iterator.raw()) };
        if page.is_null() {
            return Err(RESULT_ERROR.into());
        }
        let mut words = 0usize;
        let mut bytes = 0usize;
        let mut visited = 0usize;
        loop {
            check_interest(&cancel, started)?;
            visited += 1;
            if visited > MAX_WORDS {
                return Err(RESULT_ERROR.into());
            }
            native_trace!("word_text_begin");
            let text = Text {
                pointer: unsafe { (api.text)(iterator.raw(), 3) },
                delete: api.text_delete,
            };
            native_trace!("word_text_end");
            if !text.pointer.is_null() {
                let text = unsafe { bounded_text(text.pointer, 512 * 4 + 1)? };
                let text = text.trim();
                if !text.is_empty() {
                    if text.chars().count() > 512 || text.chars().any(|c| c.is_control()) {
                        return Err(RESULT_ERROR.into());
                    }
                    let (mut left, mut top, mut right, mut bottom) = (0, 0, 0, 0);
                    if unsafe {
                        (api.bounding)(page, 3, &mut left, &mut top, &mut right, &mut bottom)
                    } == 0
                        || left < 0
                        || top < 0
                        || right <= left
                        || bottom <= top
                        || right > geometry.width as c_int
                        || bottom > geometry.height as c_int
                    {
                        return Err(RESULT_ERROR.into());
                    }
                    if document.lines.is_empty() || unsafe { (api.beginning)(page, 2) } != 0 {
                        if document.lines.len() >= MAX_LINES {
                            return Err(RESULT_ERROR.into());
                        }
                        document.lines.push(OcrLine {
                            text: String::new(),
                            words: Vec::new(),
                        });
                    }
                    words += 1;
                    bytes = bytes.saturating_add(text.len() * 2 + 1);
                    if words > MAX_WORDS || bytes > MAX_TEXT {
                        return Err(RESULT_ERROR.into());
                    }
                    let line = document
                        .lines
                        .last_mut()
                        .ok_or_else(|| RESULT_ERROR.to_owned())?;
                    if !line.text.is_empty() {
                        line.text.push(' ');
                    }
                    line.text.push_str(text);
                    if line.text.chars().count() > 8192 {
                        return Err(RESULT_ERROR.into());
                    }
                    let (x, y, width, height) = geometry.rect(
                        left.into(),
                        top.into(),
                        (right - left).into(),
                        (bottom - top).into(),
                    );
                    line.words.push(OcrWord {
                        text: text.into(),
                        x,
                        y,
                        width,
                        height,
                    });
                }
            }
            native_trace!("iterator_next_begin");
            let next = unsafe { (api.next)(iterator.raw(), 3) };
            native_trace!("iterator_next_end");
            if next == 0 {
                break;
            }
        }
    }
    check_interest(&cancel, started)?;
    validate_ocr_document(&document).map_err(|_| RESULT_ERROR.to_owned())?;
    native_trace!("result_validated");
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba};

    #[test]
    fn abi_selection_preserves_preference_and_does_not_hide_probe_errors() {
        assert_eq!(select_abi(|_| Ok(true)).unwrap(), Abi::Five);
        assert_eq!(
            select_abi(|soname| Ok(soname == "libtesseract.so.4")).unwrap(),
            Abi::FourOne
        );
        assert!(select_abi(|_| Ok(false)).is_err());
        let mut probes = 0;
        assert!(select_abi(|_| {
            probes += 1;
            Err("unreadable preferred library".into())
        })
        .is_err());
        assert_eq!(probes, 1);
        for version in ["4.0.0", "4.2.0", "3.05.02", "6.0.0", "5.5.1"] {
            assert!(!Abi::FourOne.accepts_version(version));
        }
        assert!(Abi::FourOne.accepts_version("4.1.1"));
        assert!(!Abi::Five.accepts_version("4.1.1"));
        assert!(Abi::Five.accepts_version("5.5.1"));
        assert_eq!(
            Abi::FourOne.model_roots()[0],
            "/usr/share/tesseract-ocr/4.00/tessdata"
        );
        assert_eq!(
            Abi::Five.model_roots()[0],
            "/usr/share/tesseract-ocr/5/tessdata"
        );
    }

    #[test]
    fn cancellation_and_original_deadline_reject_before_native_resources() {
        let bitmap = RgbaImage::new(16, 16);
        assert_eq!(
            recognize_impl(bitmap, Arc::new(AtomicBool::new(true))).unwrap_err(),
            CANCEL_ERROR
        );
        let mut interest = Interest {
            cancel: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
        };
        let context = (&mut interest as *mut Interest).cast();
        assert!(!unsafe { cancelled(context, 0) });
        interest.cancel.store(true, Ordering::Release);
        assert!(unsafe { cancelled(context, 0) });
        interest.cancel.store(false, Ordering::Release);
        interest.started = Instant::now() - TIMEOUT;
        assert!(unsafe { cancelled(context, 0) });
        assert!(unsafe { cancelled(std::ptr::null_mut(), 0) });
    }

    #[test]
    fn native_text_reads_are_bounded_and_utf8_is_strict() {
        let text = CString::new("OCR 文").unwrap();
        assert_eq!(
            unsafe { bounded_text(text.as_ptr(), 10) }.unwrap(),
            "OCR 文"
        );
        assert!(unsafe { bounded_text(text.as_ptr(), 3) }.is_err());
        let invalid = [255u8, 0];
        assert!(unsafe { bounded_text(invalid.as_ptr().cast(), invalid.len()) }.is_err());
    }

    fn synthetic_english() -> PixelBuffer {
        use mewu_core::{Drawing, DrawingKind, DrawingPoint, Region};
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(1400, 240, Rgba([255, 255, 255, 255])));
        crate::drawing_render::crop_region(
            &image,
            &Region {
                id: "linux-ocr-synthetic".into(),
                x: 0.,
                y: 0.,
                width: 1400.,
                height: 240.,
                drawings: vec![Drawing {
                    id: "known-english".into(),
                    kind: DrawingKind::Text,
                    color: "#000000".into(),
                    stroke_width: 1.,
                    points: vec![DrawingPoint { x: 48., y: 64. }],
                    text: Some("Hello OCR test 12345".into()),
                    font_size: Some(64.),
                    origin: None,
                    rich: None,
                }],
                ..Default::default()
            },
        )
        .unwrap()
        .into_rgba8()
    }

    #[test]
    #[ignore = "explicit native Linux Tesseract 4.1/5/system English model test; synthetic image only, no screen/network access"]
    fn real_linux_tesseract_recognizes_known_english_and_blank() {
        struct TraceScope;
        impl Drop for TraceScope {
            fn drop(&mut self) {
                NATIVE_TEST_TRACE.with(|flag| flag.set(false));
            }
        }
        NATIVE_TEST_TRACE.with(|flag| flag.set(true));
        let _trace = TraceScope;
        native_trace!("synthetic_english_begin");
        let image = synthetic_english();
        assert!(image.pixels().filter(|pixel| pixel[0] < 128).count() > 1000);
        let document = recognize_impl(image, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(document.engine, "linux.tesseract");
        assert!(document
            .language
            .split('+')
            .any(|language| language == "eng"));
        assert_eq!((document.width, document.height), (1400, 240));
        let text: String = document
            .lines
            .iter()
            .flat_map(|line| line.text.chars())
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            text.contains("HelloOCRtest12345"),
            "known synthetic token was not recognized"
        );
        let word = document
            .lines
            .iter()
            .flat_map(|line| &line.words)
            .find(|word| word.text == "Hello")
            .expect("known word with its actual native rectangle");
        assert!(word.x >= 40. && word.x < 100. && word.width > 100. && word.height > 30.);
        validate_ocr_document(&document).unwrap();
        native_trace!("blank_begin");
        let blank = recognize_impl(
            RgbaImage::from_pixel(200, 100, Rgba([255, 255, 255, 255])),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(blank.lines.is_empty());
        assert_eq!(ACTIVE_WORKERS.load(Ordering::Acquire), 0);
    }
}
