// SPDX-License-Identifier: MPL-2.0
//! Profiles describe the image-coordinate contract, not a claim about an
//! arbitrary provider's internal vision transform. Host chooses explicitly.
use mewu_core::{
    DrawingPoint, VisualInputTarget, VisualPixelRect, VisualPixelSize, VisualSourceFence,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualCoordinateProfile {
    /// Explicit user annotation action with an unverified compatible endpoint.
    /// The host must not advertise this as a no-server-resize guarantee.
    EncodedPixelsBestEffortV1,
    /// Host has independently selected a verified native Messages adapter.
    ClaudeNativePixelsV1,
    /// Explicit app tool contract; not an inference from a model name/result.
    /// Positions use 0..1000 on each axis. Styles remain encoded-image pixels.
    ExplicitNormalized1000V2,
}
impl VisualCoordinateProfile {
    pub fn id(self) -> &'static str {
        match self {
            Self::EncodedPixelsBestEffortV1 => "encoded-pixels-best-effort-v1",
            Self::ClaudeNativePixelsV1 => "claude-native-1024-v1",
            Self::ExplicitNormalized1000V2 => "explicit-normalized1000-v2",
        }
    }
    pub(crate) fn from_id(id: &str) -> Result<Self, String> {
        match id {
            "encoded-pixels-best-effort-v1" => Ok(Self::EncodedPixelsBestEffortV1),
            "claude-native-1024-v1" => Ok(Self::ClaudeNativePixelsV1),
            "explicit-normalized1000-v2" => Ok(Self::ExplicitNormalized1000V2),
            _ => Err("批注坐标配置不受支持".into()),
        }
    }
    pub(crate) fn normalized(self) -> bool {
        matches!(self, Self::ExplicitNormalized1000V2)
    }
    /// Convert only the explicitly selected profile. Never clamp or guess from
    /// an out-of-range response; core still verifies the encoded pixel bounds.
    pub(crate) fn encoded_point(
        self,
        point: DrawingPoint,
        encoded: VisualPixelSize,
    ) -> Result<DrawingPoint, String> {
        if self.normalized() {
            if !point.x.is_finite()
                || !point.y.is_finite()
                || !(0. ..=1000.).contains(&point.x)
                || !(0. ..=1000.).contains(&point.y)
            {
                return Err("批注归一化坐标超出范围".into());
            }
            Ok(DrawingPoint {
                x: point.x * f64::from(encoded.width) / 1000.,
                y: point.y * f64::from(encoded.height) / 1000.,
            })
        } else {
            Ok(point)
        }
    }
    pub(crate) fn encoded_size(self, width: u32, height: u32) -> Result<VisualPixelSize, String> {
        if width == 0 || height == 0 {
            return Err("批注附件尺寸无效".into());
        }
        let long = width.max(height);
        let scale = long.min(1024);
        let round = |n: u32| {
            ((u64::from(n) * u64::from(scale) + u64::from(long) / 2) / u64::from(long)).max(1)
                as u32
        };
        let size = VisualPixelSize {
            width: round(width),
            height: round(height),
        };
        if matches!(self, Self::ClaudeNativePixelsV1)
            && ((size.width.div_ceil(28) * 28 > 1568)
                || (size.height.div_ceil(28) * 28 > 1568)
                || size.width.div_ceil(28) * size.height.div_ceil(28) > 1568)
        {
            return Err("批注附件超出坐标配置范围".into());
        }
        Ok(size)
    }
}
pub(crate) struct PreparedVisualSource {
    pub region_id: String,
    pub source: VisualSourceFence,
    pub crop: VisualPixelRect,
}
pub(crate) fn target(
    source: &PreparedVisualSource,
    attachment_id: String,
    attachment_ordinal: usize,
    bytes: &[u8],
    resized: VisualPixelSize,
    tile: VisualPixelRect,
    encoded: VisualPixelSize,
    profile: VisualCoordinateProfile,
) -> VisualInputTarget {
    VisualInputTarget {
        handle: uuid::Uuid::new_v4().to_string(),
        attachment_id,
        attachment_ordinal: attachment_ordinal as u32,
        attachment_sha256: format!("{:x}", Sha256::digest(bytes)),
        region_id: source.region_id.clone(),
        source: source.source.clone(),
        crop: source.crop,
        resized,
        tile,
        encoded,
        profile_id: profile.id().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_record_exact_rounded_content_sizes_not_padding() {
        let p = VisualCoordinateProfile::ClaudeNativePixelsV1;
        assert_eq!(
            p.encoded_size(2048, 1311).unwrap(),
            VisualPixelSize {
                width: 1024,
                height: 656
            }
        );
        assert_eq!(
            p.encoded_size(17, 3).unwrap(),
            VisualPixelSize {
                width: 17,
                height: 3
            }
        );
        assert!(p.encoded_size(0, 10).is_err());
        assert_ne!(
            p.id(),
            VisualCoordinateProfile::EncodedPixelsBestEffortV1.id()
        );
    }
    #[test]
    fn explicit_normalized_profile_is_bounded_without_changing_the_encoded_image_size() {
        let p = VisualCoordinateProfile::ExplicitNormalized1000V2;
        assert_eq!(VisualCoordinateProfile::from_id(p.id()).unwrap(), p);
        assert!(VisualCoordinateProfile::from_id("EXPLICIT-normalized1000-v2").is_err());
        let size = p.encoded_size(1201, 801).unwrap();
        assert_eq!(
            size,
            VisualPixelSize {
                width: 1024,
                height: 683
            }
        );
        assert_eq!(
            size,
            VisualCoordinateProfile::EncodedPixelsBestEffortV1
                .encoded_size(1201, 801)
                .unwrap()
        );
        assert_eq!(
            p.encoded_point(DrawingPoint { x: 1000., y: 1000. }, size)
                .unwrap(),
            DrawingPoint { x: 1024., y: 683. }
        );
        assert_eq!(
            p.encoded_point(DrawingPoint { x: 500., y: 500. }, size)
                .unwrap(),
            DrawingPoint { x: 512., y: 341.5 }
        );
        for value in [f64::NAN, f64::INFINITY, -0.1, 1000.1] {
            assert!(p
                .encoded_point(DrawingPoint { x: value, y: 10. }, size)
                .is_err());
            assert!(p
                .encoded_point(DrawingPoint { x: 10., y: value }, size)
                .is_err());
        }
        // A legacy result is passed to core's original bounds check, never fixed.
        assert_eq!(
            VisualCoordinateProfile::EncodedPixelsBestEffortV1
                .encoded_point(DrawingPoint { x: 856., y: 737. }, size)
                .unwrap(),
            DrawingPoint { x: 856., y: 737. }
        );
    }
}
