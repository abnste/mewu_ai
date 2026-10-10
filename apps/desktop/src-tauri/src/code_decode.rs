// SPDX-License-Identifier: MPL-2.0
//! Bounded local barcode decoding. Only the supervised code worker calls decode.
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt};

pub const MAX_PIXELS: usize = 4 * 1024 * 1024;
pub const MAX_SIDE: u32 = 8192;
pub const MAX_CODES: usize = 12;
pub const MAX_TEXT_BYTES: usize = 8 * 1024;
pub const MAX_RESULT_BYTES: usize = 64 * 1024;

pub struct CodeImage {
    pub width: u32,
    pub height: u32,
    pub luma: Vec<u8>,
}
impl CodeImage {
    pub fn validate(&self) -> Result<(), CodeError> {
        let length = pixel_count(self.width, self.height)?;
        if self.luma.len() != length {
            return Err(CodeError::InputLimit);
        }
        Ok(())
    }
}
pub fn pixel_count(width: u32, height: u32) -> Result<usize, CodeError> {
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(CodeError::InputLimit)?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || count > MAX_PIXELS {
        return Err(CodeError::InputLimit);
    }
    Ok(count)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", try_from = "String")]
pub enum CodeFormat {
    QrCode,
    DataMatrix,
    Aztec,
    Pdf417,
    Ean13,
    Ean8,
    UpcA,
    Code128,
    Code39,
}
impl TryFrom<String> for CodeFormat {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "qr_code" => Ok(Self::QrCode),
            "data_matrix" => Ok(Self::DataMatrix),
            "aztec" => Ok(Self::Aztec),
            "pdf417" => Ok(Self::Pdf417),
            "ean13" => Ok(Self::Ean13),
            "ean8" => Ok(Self::Ean8),
            "upc_a" => Ok(Self::UpcA),
            "code128" => Ok(Self::Code128),
            "code39" => Ok(Self::Code39),
            _ => Err("unsupported code format"),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DecodedCode {
    pub format: CodeFormat,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", try_from = "String")]
pub enum CodeError {
    Busy,
    Cancelled,
    TimedOut,
    InputLimit,
    ResultLimit,
    DecodeFailed,
    Protocol,
    WorkerUnavailable,
    CleanupTimedOut,
    Unsupported,
}
impl TryFrom<String> for CodeError {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "busy" => Ok(Self::Busy),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            "input_limit" => Ok(Self::InputLimit),
            "result_limit" => Ok(Self::ResultLimit),
            "decode_failed" => Ok(Self::DecodeFailed),
            "protocol" => Ok(Self::Protocol),
            "worker_unavailable" => Ok(Self::WorkerUnavailable),
            "cleanup_timed_out" => Ok(Self::CleanupTimedOut),
            "unsupported" => Ok(Self::Unsupported),
            _ => Err("unsupported code error"),
        }
    }
}
impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "识别任务尚未结束",
            Self::Cancelled => "识别已取消",
            Self::TimedOut => "识别超时",
            Self::InputLimit => "图片超出识别范围",
            Self::ResultLimit => "识别内容过多，请缩小选区",
            Self::DecodeFailed => "无法读取码内容",
            Self::Protocol | Self::WorkerUnavailable => "本机识别暂不可用",
            Self::CleanupTimedOut => "识别任务尚未停止",
            Self::Unsupported => "此平台暂不支持本机扫码",
        })
    }
}
impl std::error::Error for CodeError {}

pub fn validate_results(codes: &[DecodedCode]) -> Result<(), CodeError> {
    if codes.len() > MAX_CODES {
        return Err(CodeError::ResultLimit);
    }
    let mut seen = HashSet::new();
    for code in codes {
        if code.text.len() > MAX_TEXT_BYTES {
            return Err(CodeError::ResultLimit);
        }
        if code.text.trim().is_empty()
            || code
                .text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            || !seen.insert(code.text.as_str())
        {
            return Err(CodeError::DecodeFailed);
        }
    }
    // Includes JSON escaping/format metadata, not just raw text byte counts.
    if serde_json::to_vec(codes)
        .map_err(|_| CodeError::Protocol)?
        .len()
        > MAX_RESULT_BYTES
    {
        return Err(CodeError::ResultLimit);
    }
    Ok(())
}

fn normalize(results: Vec<rxing::RXingResult>) -> Result<Vec<DecodedCode>, CodeError> {
    use rxing::BarcodeFormat as F;
    let mut seen = HashSet::new();
    let mut codes = Vec::new();
    for result in results {
        let text = result.getText();
        if text.trim().is_empty() || !seen.insert(text.to_owned()) {
            continue;
        }
        let format = match result.getBarcodeFormat() {
            F::QR_CODE => CodeFormat::QrCode,
            F::DATA_MATRIX => CodeFormat::DataMatrix,
            F::AZTEC => CodeFormat::Aztec,
            F::PDF_417 => CodeFormat::Pdf417,
            F::EAN_13 => CodeFormat::Ean13,
            F::EAN_8 => CodeFormat::Ean8,
            F::UPC_A => CodeFormat::UpcA,
            F::CODE_128 => CodeFormat::Code128,
            F::CODE_39 => CodeFormat::Code39,
            _ => return Err(CodeError::DecodeFailed),
        };
        codes.push(DecodedCode {
            format,
            text: text.to_owned(),
        });
        validate_results(&codes)?; // Refuse the whole result; never truncate to twelve.
    }
    Ok(codes)
}

pub fn decode(image: CodeImage) -> Result<Vec<DecodedCode>, CodeError> {
    use rxing::{
        common::HybridBinarizer,
        multi::{GenericMultipleBarcodeReader, MultipleBarcodeReader},
        BarcodeFormat as F, BinaryBitmap, DecodeHints, Exceptions, Luma8Source,
        MultiUseMultiFormatReader,
    };
    image.validate()?;
    let mut hints = DecodeHints::default();
    hints.PossibleFormats = Some(HashSet::from([
        F::QR_CODE,
        F::DATA_MATRIX,
        F::AZTEC,
        F::PDF_417,
        F::EAN_13,
        F::EAN_8,
        F::UPC_A,
        F::CODE_128,
        F::CODE_39,
    ]));
    hints.TryHarder = Some(true);
    hints.AlsoInverted = Some(true);
    // Borrow original bytes for a fallback, without a second full input clone.
    // Luma8Source supports crop and 90-degree rotation; no external image reader.
    let result = {
        let source = Luma8Source::new_with_slice(&image.luma, image.width, image.height)
            .map_err(|_| CodeError::InputLimit)?;
        GenericMultipleBarcodeReader::new(MultiUseMultiFormatReader::default())
            .decode_multiple_with_hints(
                &mut BinaryBitmap::new(HybridBinarizer::new(source)),
                &hints,
            )
    };
    match result {
        Ok(results) => normalize(results),
        Err(Exceptions::NotFoundException(_)) => {
            match rxing::helpers::detect_in_luma_slice_with_hints(
                &image.luma,
                image.width,
                image.height,
                None,
                &mut hints,
            ) {
                Ok(result) => normalize(vec![result]),
                Err(Exceptions::NotFoundException(_)) => Ok(Vec::new()),
                Err(_) => Err(CodeError::DecodeFailed),
            }
        }
        Err(_) => Err(CodeError::DecodeFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encoded(format: rxing::BarcodeFormat, text: &str) -> CodeImage {
        use rxing::Writer;
        let hints = rxing::EncodeHints {
            CharacterSet: Some("UTF-8".into()),
            ..Default::default()
        };
        let matrix = rxing::MultiFormatWriter::default()
            .encode_with_hints(text, &format, 240, 180, &hints)
            .unwrap();
        let width = matrix.getWidth() + 32;
        let height = matrix.getHeight() + 32;
        let mut luma = vec![255; (width * height) as usize];
        for y in 0..matrix.getHeight() {
            for x in 0..matrix.getWidth() {
                luma[((y + 16) * width + x + 16) as usize] = if matrix.get(x, y) { 0 } else { 255 };
            }
        }
        CodeImage {
            width,
            height,
            luma,
        }
    }
    fn rotate(image: CodeImage) -> CodeImage {
        let mut luma = vec![0; image.luma.len()];
        for y in 0..image.height {
            for x in 0..image.width {
                luma[(x * image.height + image.height - 1 - y) as usize] =
                    image.luma[(y * image.width + x) as usize];
            }
        }
        CodeImage {
            width: image.height,
            height: image.width,
            luma,
        }
    }
    #[test]
    fn synthetic_nine_formats_decode_as_plain_text() {
        use rxing::BarcodeFormat as F;
        for (format, expected, text) in [
            (F::QR_CODE, CodeFormat::QrCode, "MEWU QR 123"),
            (F::DATA_MATRIX, CodeFormat::DataMatrix, "MEWU MATRIX 123"),
            (F::AZTEC, CodeFormat::Aztec, "MEWU AZTEC 123"),
            (F::PDF_417, CodeFormat::Pdf417, "MEWU PDF417 123"),
            (F::EAN_13, CodeFormat::Ean13, "5901234123457"),
            (F::EAN_8, CodeFormat::Ean8, "96385074"),
            (F::UPC_A, CodeFormat::UpcA, "036000291452"),
            (F::CODE_128, CodeFormat::Code128, "Mewu128"),
            (F::CODE_39, CodeFormat::Code39, "MEWU123"),
        ] {
            let codes = decode(encoded(format, text)).unwrap();
            assert!(
                codes
                    .iter()
                    .any(|code| code.format == expected && code.text == text),
                "{format:?}: {codes:?}"
            );
        }
    }
    #[test]
    fn chinese_rotation_inversion_and_complete_redaction() {
        for format in [
            rxing::BarcodeFormat::QR_CODE,
            rxing::BarcodeFormat::CODE_128,
        ] {
            let text = if format == rxing::BarcodeFormat::QR_CODE {
                "本地合成二维码 https://example.invalid/中文"
            } else {
                "ROTATE-128"
            };
            let mut image = rotate(encoded(format, text));
            for pixel in &mut image.luma {
                *pixel = 255 - *pixel;
            }
            assert!(decode(image).unwrap().iter().any(|code| code.text == text));
        }
        let mut redacted = encoded(rxing::BarcodeFormat::QR_CODE, "redacted-value");
        redacted.luma.fill(128);
        assert!(decode(redacted).unwrap().is_empty());
    }
    #[test]
    fn multiple_qr_codes_deduplicate_exact_payload_without_trimming() {
        let source = ["  first payload  ", "第二个内容", "  first payload  "]
            .map(|text| encoded(rxing::BarcodeFormat::QR_CODE, text));
        let width = source.iter().map(|image| image.width + 40).sum::<u32>();
        let height = source.iter().map(|image| image.height).max().unwrap();
        let mut luma = vec![255; (width * height) as usize];
        let mut left = 0;
        for image in source {
            for y in 0..image.height {
                let at = (y * width + left) as usize;
                let from = (y * image.width) as usize;
                luma[at..at + image.width as usize]
                    .copy_from_slice(&image.luma[from..from + image.width as usize]);
            }
            left += image.width + 40;
        }
        let codes = decode(CodeImage {
            width,
            height,
            luma,
        })
        .unwrap();
        assert_eq!(codes.len(), 2);
        assert!(codes.iter().any(|code| code.text == "  first payload  "));
        assert!(codes.iter().any(|code| code.text == "第二个内容"));
    }
    #[test]
    fn dimensions_and_result_limits_are_whole_document() {
        assert_eq!(pixel_count(2048, 2048), Ok(MAX_PIXELS));
        for (w, h) in [(0, 1), (1, 0), (8193, 1), (2049, 2048)] {
            assert!(pixel_count(w, h).is_err());
        }
        assert!(CodeImage {
            width: 2,
            height: 2,
            luma: vec![0; 3]
        }
        .validate()
        .is_err());
        let code = DecodedCode {
            format: CodeFormat::QrCode,
            text: "中".repeat(2731),
        };
        assert_eq!(validate_results(&[code]), Err(CodeError::ResultLimit));
        let codes: Vec<_> = (0..13)
            .map(|i| DecodedCode {
                format: CodeFormat::QrCode,
                text: i.to_string(),
            })
            .collect();
        assert_eq!(validate_results(&codes), Err(CodeError::ResultLimit));
    }
    #[test]
    fn wire_is_strict_and_payload_is_never_normalized() {
        assert!(serde_json::from_str::<CodeFormat>(r#"{"qr_code":null}"#).is_err());
        assert!(serde_json::from_str::<DecodedCode>(
            r#"{"format":"qr_code","text":"x","url":"x"}"#
        )
        .is_err());
        let text = "  https://example.invalid/?a=中\n".to_owned();
        let code = DecodedCode {
            format: CodeFormat::QrCode,
            text: text.clone(),
        };
        validate_results(&[code.clone()]).unwrap();
        assert_eq!(code.text, text);
        assert!(validate_results(&[code.clone(), code]).is_err());
    }
    #[test]
    fn blank_is_a_successful_negative() {
        assert_eq!(
            decode(CodeImage {
                width: 64,
                height: 64,
                luma: vec![255; 4096]
            }),
            Ok(vec![])
        );
    }
}
