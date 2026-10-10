// SPDX-License-Identifier: MPL-2.0
//! Read-only, remote-origin document transport. This is deliberately not a Tauri
//! custom protocol: registered protocols can be classified as local IPC origins.
use mewu_core::{Asset, AssetKind};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};
use tauri::{AppHandle, Manager};
use tiny_http::{Header, Response, ResponseBox, Server, StatusCode};

const DOCUMENT_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-scripts";
const MAX_IMAGE_BYTES: u64 = 128 * 1024 * 1024;

pub struct ContentServer {
    server: Server,
    authority: String,
}

pub fn bind() -> Result<ContentServer, String> {
    let server = Server::http((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| "无法启动内容视图".to_string())?;
    let address = server.server_addr().to_ip().ok_or("内容视图地址无效")?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("内容视图地址无效".into());
    }
    Ok(ContentServer {
        server,
        authority: address.to_string(),
    })
}

impl ContentServer {
    pub fn origin(&self) -> String {
        format!("http://{}", self.authority)
    }

    /// Run on a dedicated background thread after Host has been managed by Tauri.
    /// The caller must not grant remote IPC capabilities to this origin. Some
    /// WebView runtimes inject Tauri internals into frames; absence of a JS global
    /// is not the security boundary. The native ACL must reject every such call.
    pub fn serve(self, app: AppHandle) {
        let server = Arc::new(self);
        // A video response can remain open while playback is paused. Keep disk
        // and socket work off the UI thread without spawning per-request threads.
        for _ in 0..3 {
            let worker = Arc::clone(&server);
            let app = app.clone();
            std::thread::spawn(move || worker.serve_requests(app));
        }
        server.serve_requests(app);
    }

    fn serve_requests(&self, app: AppHandle) {
        for request in self.server.incoming_requests() {
            let hosts: Vec<&str> = request
                .headers()
                .iter()
                .filter(|header| header.field.equiv("Host"))
                .map(|header| header.value.as_str())
                .collect();
            let result = (|| -> Option<ResponseBox> {
                if !request
                    .remote_addr()
                    .is_some_and(|address| address.ip().is_loopback())
                {
                    return None;
                }
                let id = validated_id(
                    request.method().as_str(),
                    &hosts,
                    request.url(),
                    &self.authority,
                )?;
                let host = app.state::<super::Host>();
                // Drop the state mutex before disk I/O or sending any response.
                let asset = {
                    let engine = host.lock().ok()?;
                    super::assets::resolve(&engine.store.snapshot(), id)
                };
                let range = video_range_header(request.method().as_str(), request.headers());
                if let Some(asset) = asset {
                    asset_response(&asset, &host.assets, range)
                } else {
                    // Closing a pin immediately revokes new reads. An already
                    // opened response owns a lease until its file handle closes.
                    let lease = super::pin_host::resolve_asset(&app, id)?;
                    let asset = lease.asset().clone();
                    let root = lease.root().to_owned();
                    image_response(&asset, &root, Some(lease))
                }
            })();
            let response = result.unwrap_or_else(|| {
                document_response(404, Vec::new(), "text/plain; charset=utf-8").boxed()
            });
            // No access log: UUIDs identify private local documents.
            let _ = request.respond(response);
        }
    }
}

struct LeasedImageReader {
    // Fields drop in declaration order: release Windows' file handle before
    // the last lease attempts to remove its own temporary PNG.
    reader: std::io::Take<File>,
    _lease: Option<super::pin_host::PinAssetLease>,
}
impl Read for LeasedImageReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buffer)
    }
}

fn image_response(
    asset: &Asset,
    root: &Path,
    lease: Option<super::pin_host::PinAssetLease>,
) -> Option<ResponseBox> {
    let path = super::assets::verified_path(asset, root).ok()?;
    let mut file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    let length = metadata.len();
    if !metadata.is_file() || !(12..=MAX_IMAGE_BYTES).contains(&length) {
        return None;
    }
    let mut signature = [0_u8; 12];
    file.read_exact(&mut signature).ok()?;
    let mime = image_mime(&signature)?;
    file.seek(SeekFrom::Start(0)).ok()?;
    // Bound the opened handle, including if another process grows the file.
    // tiny_http streams this reader without allocating the whole image.
    let response = Response::new(
        StatusCode(200),
        Vec::new(),
        LeasedImageReader {
            reader: file.take(length),
            _lease: lease,
        },
        Some(length as usize),
        None,
    );
    Some(with_headers(response, mime).boxed())
}

fn asset_response(asset: &Asset, root: &Path, range: Option<&str>) -> Option<ResponseBox> {
    match asset.kind {
        AssetKind::Image => image_response(asset, root, None),
        AssetKind::Video => {
            let (mut file, length) = super::assets::open_video(asset, root).ok()?;
            let (status, start, count, content_range) = match byte_range(range, length) {
                ByteRange::Full => (200, 0, length, None),
                ByteRange::Partial { start, end } => (
                    206,
                    start,
                    end - start + 1,
                    Some(format!("bytes {start}-{end}/{length}")),
                ),
                ByteRange::Unsatisfiable => {
                    let mut response = document_response(416, Vec::new(), "video/mp4");
                    add_range_headers(&mut response, Some(format!("bytes */{length}")));
                    return Some(response.boxed());
                }
            };
            file.seek(SeekFrom::Start(start)).ok()?;
            let mut response = with_headers(
                Response::new(
                    StatusCode(status),
                    Vec::new(),
                    file.take(count),
                    Some(count.try_into().ok()?),
                    None,
                )
                .with_chunked_threshold(usize::MAX),
                "video/mp4",
            );
            add_range_headers(&mut response, content_range);
            Some(response.boxed())
        }
        AssetKind::Html | AssetKind::Svg => {
            let mime = if matches!(asset.kind, AssetKind::Html) {
                "text/html; charset=utf-8"
            } else {
                "image/svg+xml; charset=utf-8"
            };
            let body = super::assets::read_text(asset, root).ok()?;
            if body.len() > super::assets::MAX_TEXT {
                return None;
            }
            Some(document_response(200, body.into_bytes(), mime).boxed())
        }
        AssetKind::File | AssetKind::Text => None,
    }
}

fn add_range_headers<R: Read>(response: &mut Response<R>, content_range: Option<String>) {
    response.add_header(Header::from_bytes("Accept-Ranges", "bytes").unwrap());
    if let Some(value) = content_range {
        response.add_header(Header::from_bytes("Content-Range", value).unwrap());
    }
}

fn video_range_header<'a>(method: &str, headers: &'a [Header]) -> Option<&'a str> {
    // We do not publish validators. An If-Range request cannot be validated, so
    // send the complete representation. HEAD always describes the full GET.
    if method != "GET" || headers.iter().any(|h| h.field.equiv("If-Range")) {
        return None;
    }
    let mut ranges = headers.iter().filter(|h| h.field.equiv("Range"));
    let first = ranges.next()?;
    if ranges.next().is_some() {
        return None;
    }
    Some(first.value.as_str())
}

#[derive(Debug, PartialEq, Eq)]
enum ByteRange {
    Full,
    Partial { start: u64, end: u64 },
    Unsatisfiable,
}

fn byte_range(value: Option<&str>, length: u64) -> ByteRange {
    let Some(value) = value else {
        return ByteRange::Full;
    };
    // RFC 9110 allows ignoring unsupported multi-range or malformed requests.
    // Bound parsing work and never convert an overflowing decimal into an offset.
    if value.len() > 128 || length == 0 {
        return ByteRange::Full;
    }
    let Some((unit, value)) = value.trim().split_once('=') else {
        return ByteRange::Full;
    };
    if !unit.eq_ignore_ascii_case("bytes") || value.contains(',') {
        return ByteRange::Full;
    }
    let Some((start, end)) = value.split_once('-') else {
        return ByteRange::Full;
    };
    let decimal = |text: &str| -> Option<u64> {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        text.parse().ok()
    };
    if start.is_empty() {
        let Some(suffix) = decimal(end) else {
            return ByteRange::Full;
        };
        return if suffix == 0 {
            ByteRange::Unsatisfiable
        } else {
            ByteRange::Partial {
                start: length.saturating_sub(suffix),
                end: length - 1,
            }
        };
    }
    let Some(start) = decimal(start) else {
        return ByteRange::Full;
    };
    let end = if end.is_empty() {
        length - 1
    } else {
        let Some(end) = decimal(end) else {
            return ByteRange::Full;
        };
        if end < start {
            return ByteRange::Full;
        }
        end.min(length - 1)
    };
    if start >= length {
        ByteRange::Unsatisfiable
    } else {
        ByteRange::Partial { start, end }
    }
}

fn image_mime(signature: &[u8; 12]) -> Option<&'static str> {
    if signature.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if signature.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if signature.starts_with(b"RIFF") && &signature[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Accept only the canonical UUID path and one exact numeric Host. In particular,
/// never URL-decode or normalize attacker-controlled path segments into filenames.
fn validated_id<'a>(
    method: &str,
    hosts: &[&str],
    target: &'a str,
    authority: &str,
) -> Option<&'a str> {
    if !matches!(method, "GET" | "HEAD") || hosts != [authority] {
        return None;
    }
    let id = target.strip_prefix('/')?;
    if id.len() != 36 {
        return None;
    }
    let parsed = uuid::Uuid::parse_str(id).ok()?;
    if parsed.hyphenated().to_string() != id {
        return None;
    }
    Some(id)
}

fn document_response(status: u16, body: Vec<u8>, mime: &str) -> Response<Cursor<Vec<u8>>> {
    with_headers(Response::from_data(body).with_status_code(status), mime)
}

fn with_headers<R: Read>(mut response: Response<R>, mime: &str) -> Response<R> {
    // Deliberately no Access-Control-Allow-* headers, directory index or redirect.
    for (name, value) in [
        ("Content-Type", mime),
        ("Content-Security-Policy", DOCUMENT_CSP),
        ("X-Content-Type-Options", "nosniff"),
        ("Cache-Control", "no-store"),
        ("Referrer-Policy", "no-referrer"),
        ("Permissions-Policy", "camera=(), microphone=(), geolocation=(), clipboard-read=(), clipboard-write=(), display-capture=(), usb=(), serial=(), hid=(), payment=()"),
    ] {
        response.add_header(Header::from_bytes(name, value).expect("static content response header"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    const HOST: &str = "127.0.0.1:49321";
    const ID: &str = "f0676510-842f-44e7-b507-136e480e6f02";

    #[test]
    fn canonical_document_get_is_accepted() {
        assert_eq!(
            validated_id("GET", &[HOST], &format!("/{ID}"), HOST),
            Some(ID)
        );
        assert_eq!(
            validated_id("HEAD", &[HOST], &format!("/{ID}"), HOST),
            Some(ID)
        );
    }

    #[test]
    fn method_and_host_cannot_be_aliased_or_duplicated() {
        let target = format!("/{ID}");
        for method in ["POST", "OPTIONS", "CONNECT", "get"] {
            assert!(validated_id(method, &[HOST], &target, HOST).is_none());
        }
        for hosts in [
            vec![],
            vec![HOST, HOST],
            vec!["localhost:49321"],
            vec!["127.0.0.1"],
            vec!["127.0.0.1:49322"],
            vec!["attacker.invalid:49321"],
            vec!["127.0.0.1:49321, attacker.invalid"],
        ] {
            assert!(validated_id("GET", &hosts, &target, HOST).is_none());
        }
    }

    #[test]
    fn document_path_rejects_query_traversal_encoding_and_alternate_uuid_forms() {
        for target in [
            format!("/{ID}?download"),
            format!("/{ID}#part"),
            format!("/{ID}/"),
            format!("//{ID}"),
            format!("/../{ID}"),
            format!("/%2f{ID}"),
            format!("/{}", ID.to_uppercase()),
            format!("/{}", ID.replace('-', "")),
            format!("/{{{ID}}}"),
            format!("http://{HOST}/{ID}"),
            "/".into(),
            "/favicon.ico".into(),
            "/C:/Windows/win.ini".into(),
        ] {
            assert!(
                validated_id("GET", &[HOST], &target, HOST).is_none(),
                "accepted {target}"
            );
        }
    }

    #[test]
    fn error_and_document_responses_keep_the_sandbox_without_cors() {
        for status in [200, 404] {
            let response = document_response(status, Vec::new(), "text/html; charset=utf-8");
            let headers = response.headers();
            let csp = headers
                .iter()
                .find(|header| header.field.equiv("Content-Security-Policy"))
                .unwrap();
            assert_eq!(csp.value.as_str(), DOCUMENT_CSP);
            assert!(headers
                .iter()
                .any(|header| header.field.equiv("Cache-Control")
                    && header.value.as_str() == "no-store"));
            assert!(!headers
                .iter()
                .any(|header| header.field.equiv("Access-Control-Allow-Origin")));
            assert!(!DOCUMENT_CSP.contains("allow-same-origin"));
        }
    }

    #[test]
    fn registered_image_stream_uses_magic_and_rejects_non_images_and_file_assets() {
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(
            std::env::temp_dir().join(format!("mewu-content-test-{}", uuid::Uuid::new_v4())),
        );
        std::fs::create_dir_all(&fixture.0).unwrap();
        let path = fixture.0.join("misleading.html");
        let bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        std::fs::write(&path, bytes).unwrap();
        let mut asset = Asset {
            id: ID.into(),
            name: "misleading.html".into(),
            kind: AssetKind::Image,
            path: path.to_string_lossy().into_owned(),
            width: None,
            height: None,
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        let response = asset_response(&asset, &fixture.0, None).unwrap();
        assert!(response
            .headers()
            .iter()
            .any(|h| h.field.equiv("Content-Type") && h.value.as_str() == "image/png"));
        assert_eq!(response.data_length(), Some(bytes.len()));
        let mut streamed = Vec::new();
        response.into_reader().read_to_end(&mut streamed).unwrap();
        assert_eq!(streamed, bytes);
        asset.kind = AssetKind::File;
        assert!(asset_response(&asset, &fixture.0, None).is_none());
        asset.kind = AssetKind::Text;
        std::fs::write(&path, b"<script>literal text only</script>").unwrap();
        assert!(asset_response(&asset, &fixture.0, None).is_none());
        asset.kind = AssetKind::Image;
        std::fs::write(&path, b"<html>not an image</html>").unwrap();
        assert!(asset_response(&asset, &fixture.0, None).is_none());
        assert_eq!(image_mime(b"RIFF\x04\x00\x00\x00WEBP"), Some("image/webp"));
        assert_eq!(image_mime(b"\xff\xd8\xff\xe0abcdefgh"), Some("image/jpeg"));
    }

    #[test]
    fn single_ranges_cover_seeking_suffix_clamping_and_invalid_input() {
        for (input, start, end) in [
            ("bytes=0-0", 0, 0),
            ("bytes=10-19", 10, 19),
            ("bytes=90-200", 90, 99),
            ("bytes=90-", 90, 99),
            ("bytes=-5", 95, 99),
            ("bytes=-999", 0, 99),
            ("BYTES=0-1", 0, 1),
        ] {
            assert_eq!(
                byte_range(Some(input), 100),
                ByteRange::Partial { start, end },
                "{input}"
            );
        }
        for input in ["bytes=100-", "bytes=101-200", "bytes=-0"] {
            assert_eq!(byte_range(Some(input), 100), ByteRange::Unsatisfiable);
        }
        for input in [
            "items=0-1",
            "bytes=0-1,4-5",
            "bytes=20-10",
            "bytes=",
            "bytes=+0-1",
            "bytes=0 -1",
            "bytes=18446744073709551616-",
            "bytes=0-18446744073709551616",
        ] {
            assert_eq!(byte_range(Some(input), 100), ByteRange::Full, "{input}");
        }
        assert_eq!(byte_range(None, 100), ByteRange::Full);
        assert_eq!(byte_range(Some("bytes=0-"), 0), ByteRange::Full);
    }

    #[test]
    fn head_duplicate_ranges_and_unvalidated_if_range_use_full_response() {
        let range = Header::from_bytes("Range", "bytes=1-2").unwrap();
        assert_eq!(
            video_range_header("GET", &[range.clone()]),
            Some("bytes=1-2")
        );
        assert_eq!(video_range_header("HEAD", &[range.clone()]), None);
        assert_eq!(
            video_range_header("GET", &[range.clone(), range.clone()]),
            None
        );
        assert_eq!(
            video_range_header(
                "GET",
                &[range, Header::from_bytes("If-Range", "\"old\"").unwrap()]
            ),
            None
        );
    }

    #[test]
    fn video_range_stream_and_head_have_correct_wire_lengths() {
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join(format!("{ID}.mp4")));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let fixture =
            Fixture(std::env::temp_dir().join(format!("mewu-video-http-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir(&fixture.0).unwrap();
        let path = fixture.0.join(format!("{ID}.mp4"));
        let mut bytes = b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom\0\0\0\x08mdat".to_vec();
        bytes.resize(70_000, 0x51);
        std::fs::write(&path, &bytes).unwrap();
        let asset = Asset {
            id: ID.into(),
            name: "fixture.mp4".into(),
            kind: AssetKind::Video,
            path: path.to_string_lossy().into_owned(),
            width: None,
            height: None,
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        let response = asset_response(&asset, &fixture.0, Some("bytes=4-7")).unwrap();
        assert_eq!(response.status_code(), StatusCode(206));
        assert_eq!(response.data_length(), Some(4));
        assert!(response
            .headers()
            .iter()
            .any(|h| h.field.equiv("Content-Range") && h.value.as_str() == "bytes 4-7/70000"));
        assert!(!response
            .headers()
            .iter()
            .any(|h| h.field.equiv("Access-Control-Allow-Origin")));
        let mut body = Vec::new();
        response.into_reader().read_to_end(&mut body).unwrap();
        assert_eq!(body, b"ftyp");
        let response = asset_response(&asset, &fixture.0, Some("bytes=70000-")).unwrap();
        assert_eq!(response.status_code(), StatusCode(416));
        assert_eq!(response.data_length(), Some(0));
        assert!(response
            .headers()
            .iter()
            .any(|h| h.field.equiv("Content-Range") && h.value.as_str() == "bytes */70000"));
        let mut wire = Vec::new();
        asset_response(&asset, &fixture.0, None)
            .unwrap()
            .raw_print(&mut wire, tiny_http::HTTPVersion(1, 1), &[], true, None)
            .unwrap();
        let headers = String::from_utf8(wire).unwrap();
        assert!(headers.contains("Content-Length: 70000\r\n"));
        assert!(!headers.contains("Transfer-Encoding"));
        assert!(headers.ends_with("\r\n\r\n"));
    }
}
