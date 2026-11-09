// SPDX-License-Identifier: MPL-2.0
//! Bounded clipboard publication on a worker, never on the window thread.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const RETRY_FOR: Duration = Duration::from_millis(600);
const RETRY_INTERVAL: Duration = Duration::from_millis(15);
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn authority(check: impl Fn() -> Result<(), String>) -> impl Fn() -> Result<(), String> {
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    move || {
        if GENERATION.load(Ordering::Acquire) != generation {
            return Err("复制请求已被新的复制操作替代".into());
        }
        check()
    }
}

fn publish(
    check: impl Fn() -> Result<(), String>,
    mut write: impl FnMut() -> Result<(), arboard::Error>,
    failure: &str,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        check()?;
        match write() {
            Ok(()) => return Ok(()),
            Err(arboard::Error::ClipboardOccupied) if started.elapsed() < RETRY_FOR => {
                std::thread::sleep(RETRY_INTERVAL);
            }
            Err(arboard::Error::ClipboardOccupied) => {
                return Err("剪贴板正被其他程序占用，请稍后重试".into())
            }
            Err(_) => return Err(failure.into()),
        }
    }
}

pub fn text(value: &str, check: impl Fn() -> Result<(), String>) -> Result<(), String> {
    publish(
        authority(check),
        || arboard::Clipboard::new()?.set_text(value),
        "无法复制文字",
    )
}

pub fn image(
    value: &image::RgbaImage,
    check: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    publish(
        authority(check),
        || {
            arboard::Clipboard::new()?.set_image(arboard::ImageData {
                width: value.width() as usize,
                height: value.height() as usize,
                bytes: std::borrow::Cow::Borrowed(value.as_raw()),
            })
        },
        "无法复制图片",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn transient_occupation_is_retried_but_other_failures_are_not() {
        let calls = Cell::new(0);
        publish(
            || Ok(()),
            || {
                calls.set(calls.get() + 1);
                if calls.get() < 4 {
                    Err(arboard::Error::ClipboardOccupied)
                } else {
                    Ok(())
                }
            },
            "failed",
        )
        .unwrap();
        assert_eq!(calls.get(), 4);
        calls.set(0);
        assert_eq!(
            publish(
                || Ok(()),
                || {
                    calls.set(calls.get() + 1);
                    Err(arboard::Error::ConversionFailure)
                },
                "failed"
            ),
            Err("failed".into())
        );
        assert_eq!(calls.get(), 1);
    }
    #[test]
    fn closed_owner_stops_retry_without_publishing() {
        let calls = Cell::new(0);
        let result = publish(
            || {
                if calls.get() == 0 {
                    Ok(())
                } else {
                    Err("closed".into())
                }
            },
            || {
                calls.set(calls.get() + 1);
                Err(arboard::Error::ClipboardOccupied)
            },
            "failed",
        );
        assert_eq!(result, Err("closed".into()));
        assert_eq!(calls.get(), 1);
    }
    #[test]
    fn newer_copy_revokes_an_older_waiting_publication() {
        let older = authority(|| Ok(()));
        assert!(older().is_ok());
        let newer = authority(|| Ok(()));
        assert!(older().is_err());
        assert!(newer().is_ok());
    }
    #[test]
    fn persistent_occupation_returns_a_bounded_failure() {
        let started = Instant::now();
        assert!(publish(
            || Ok(()),
            || Err(arboard::Error::ClipboardOccupied),
            "failed"
        )
        .unwrap_err()
        .contains("占用"));
        assert!(started.elapsed() >= RETRY_FOR);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    #[ignore = "Writes synthetic text/image to the interactive Windows clipboard"]
    fn real_clipboard_recovers_after_another_thread_releases_it() {
        fn hold() -> std::thread::JoinHandle<()> {
            let (send, receive) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                use windows_sys::Win32::System::DataExchange::{CloseClipboard, OpenClipboard};
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    CreateWindowExW, DestroyWindow, WS_POPUP,
                };
                let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
                let window = unsafe {
                    CreateWindowExW(
                        0,
                        class.as_ptr(),
                        std::ptr::null(),
                        WS_POPUP,
                        0,
                        0,
                        1,
                        1,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    )
                };
                assert!(!window.is_null());
                struct Lease(windows_sys::Win32::Foundation::HWND);
                impl Drop for Lease {
                    fn drop(&mut self) {
                        unsafe {
                            CloseClipboard();
                            DestroyWindow(self.0);
                        }
                    }
                }
                let started = Instant::now();
                while unsafe { OpenClipboard(window) } == 0 {
                    assert!(started.elapsed() < Duration::from_secs(2));
                    std::thread::sleep(Duration::from_millis(10));
                }
                let _lease = Lease(window);
                send.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(200));
            });
            receive.recv_timeout(Duration::from_secs(3)).unwrap();
            worker
        }
        let holder = hold();
        // The previous one-shot implementation fails during this same hold.
        let previous = arboard::Clipboard::new().unwrap().set_text("MEWU OLD COPY");
        assert!(
            matches!(previous, Err(arboard::Error::ClipboardOccupied)),
            "unexpected previous result: {previous:?}"
        );
        text("MEWU CLIPBOARD RETRY TEST", || Ok(())).unwrap();
        holder.join().unwrap();
        assert_eq!(
            arboard::Clipboard::new().unwrap().get_text().unwrap(),
            "MEWU CLIPBOARD RETRY TEST"
        );
        let holder = hold();
        let pixels = image::RgbaImage::from_pixel(31, 17, image::Rgba([238, 72, 72, 255]));
        image(&pixels, || Ok(())).unwrap();
        holder.join().unwrap();
        let copied = arboard::Clipboard::new()
            .unwrap()
            .get_image()
            .unwrap()
            .to_owned();
        assert_eq!((copied.width, copied.height), (31, 17));
        assert_eq!(copied.bytes.as_ref(), pixels.as_raw());
    }
}
