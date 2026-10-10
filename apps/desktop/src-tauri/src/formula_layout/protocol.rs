// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontFaceIdentity {
    pub bytes: u64,
    pub sha256: String,
    pub face_index: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontIdentity {
    pub katex_inventory_sha256: String,
    pub primary: Option<FontFaceIdentity>,
    pub secondary: Option<FontFaceIdentity>,
    pub emoji: Option<FontFaceIdentity>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StyleIdentity {
    pub version: u8,
    pub color: String,
    pub font_size: f64,
    pub math_style: String,
    pub svg_color_syntax: String,
    pub dpi: u16,
    pub padding: u8,
    pub stroke_width: f64,
}
impl StyleIdentity {
    pub fn for_input(input: &FormulaInput) -> Self {
        Self {
            version: 1,
            color: input.color.clone(),
            font_size: input.font_size,
            math_style: "display".into(),
            svg_color_syntax: "rgb".into(),
            dpi: 72,
            padding: 10,
            stroke_width: 1.5,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Config {
    pub request_id: String,
    pub input: FormulaInput,
    pub font_directory: PathBuf,
    pub unicode_font: PathBuf,
    pub primary_sha256: String,
    pub output_directory: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Identity {
    pub renderer_id: String,
    pub fonts: FontIdentity,
    pub style: StyleIdentity,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Metrics {
    pub svg_bytes: usize,
    pub svg_sha256: String,
    pub view_box: [f64; 4],
    pub width: u32,
    pub height: u32,
    pub glyph_count: usize,
    pub nonspace_glyph_count: usize,
    pub path_count: usize,
    pub missing_glyphs: Vec<u32>,
    pub png_bytes: usize,
    pub png_sha256: String,
    pub rgba_sha256: String,
    pub ink_bounds: [u32; 4],
    pub touches_canvas_edge: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RenderResult {
    pub request_id: String,
    pub status: String,
    pub elapsed_ms: u64,
    pub accepted_input_chars: usize,
    pub metrics: Option<Metrics>,
    pub missing_glyphs: Vec<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Event {
    Ready { identity: Identity },
    Result { result: RenderResult },
    Done {},
}
pub(super) fn elapsed_millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}
pub(super) fn decode_frame(bytes: &[u8]) -> Result<Event, String> {
    if bytes.len() > MAX_FRAME || bytes.last() != Some(&b'\n') {
        return Err("formula_protocol_frame".into());
    }
    // Never skip non-JSON upstream stdout, and never include raw bytes/TeX in errors.
    serde_json::from_slice(bytes).map_err(|_| "formula_protocol_json".into())
}
pub(super) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_tagged_result_roundtrips_full_u64_range() {
        for elapsed in [0, 1, u32::MAX as u64, u64::MAX] {
            let event = Event::Result {
                result: RenderResult {
                    request_id: uuid::Uuid::nil().to_string(),
                    status: "parse_error".into(),
                    elapsed_ms: elapsed,
                    accepted_input_chars: 1,
                    metrics: None,
                    missing_glyphs: vec![],
                },
            };
            let mut bytes = serde_json::to_vec(&event).unwrap();
            bytes.push(b'\n');
            assert_eq!(decode_frame(&bytes).unwrap(), event);
        }
        assert_eq!(
            elapsed_millis(Duration::new(u64::MAX, 999_999_999)),
            u64::MAX
        );
    }
    #[test]
    fn logs_extra_fields_and_truncated_frames_fail_closed() {
        for bytes in [
            b"upstream log\n".as_slice(),
            b"{\"type\":\"done\",\"extra\":1}\n",
            b"{\"type\":\"done\"}",
            b"{\"type\":\"done\"}\n{}\n",
        ] {
            assert!(decode_frame(bytes).is_err());
        }
    }
}
