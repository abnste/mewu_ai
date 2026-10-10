// SPDX-License-Identifier: MPL-2.0
//! Actual PNG encode/decode for the shared drawing compositor,
//! synthetic pixels only; no user database, screen, model or clipboard.
use super::*;
use crate::rich_annotation_host::{decode, RichRaster, RichRasters};
use image::{GenericImageView, ImageEncoder, RgbaImage};
use mewu_core::{
    RichContent, RichKind, RichRendererIdentity, VerifiedRichLayout, VisualDrawingOrigin,
};

fn layout(pixels: RgbaImage) -> VerifiedRichLayout {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    VerifiedRichLayout::new(
        uuid::Uuid::new_v4().to_string(),
        RichContent::Formula {
            version: 1,
            tex: "x".into(),
            color: "#112233".into(),
            font_size: 16.,
        },
        RichRendererIdentity {
            id: "synthetic-png-fixture".into(),
            version: "1".into(),
            font_sha256: "a".repeat(64),
            style_version: 1,
        },
        pixels.width(),
        pixels.height(),
        png,
    )
    .unwrap()
}
fn rich(layout: &VerifiedRichLayout, points: [(f64, f64); 2]) -> Drawing {
    let id = || uuid::Uuid::new_v4().to_string();
    Drawing {
        id: id(),
        kind: DrawingKind::Rich,
        color: mewu_core::RICH_DRAWING_COLOR.into(),
        stroke_width: mewu_core::RICH_DRAWING_STROKE_WIDTH,
        points: points
            .into_iter()
            .map(|(x, y)| DrawingPoint { x, y })
            .collect(),
        text: None,
        font_size: None,
        rich: Some(layout.reference().clone()),
        // Geometric compositor fixture only. Source/receipt validation is
        // independently exercised in native mixed SQLite batch tests.
        origin: Some(VisualDrawingOrigin {
            run_id: id(),
            user_message_id: id(),
            tool_event_id: id(),
            group_id: id(),
            target_handle: id(),
            manifest_sha256: "b".repeat(64),
        }),
    }
}
fn line(color: &str, y: f64) -> Drawing {
    Drawing {
        id: uuid::Uuid::new_v4().to_string(),
        kind: DrawingKind::Line,
        color: color.into(),
        stroke_width: 2.,
        points: vec![DrawingPoint { x: 4., y }, DrawingPoint { x: 30., y }],
        text: None,
        font_size: None,
        origin: None,
        rich: None,
    }
}
fn region(drawings: Vec<Drawing>, x: f64, y: f64, width: f64, height: f64) -> Region {
    Region {
        x,
        y,
        width,
        height,
        drawings,
        ..Default::default()
    }
}
fn source() -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 24, Rgba([20, 40, 100, 255])))
}
fn map(layouts: &[&VerifiedRichLayout]) -> RichRasters {
    layouts
        .iter()
        .map(|layout| {
            (
                layout.reference().layout_id.clone(),
                RichRaster {
                    reference: layout.reference().clone(),
                    pixels: decode(layout).unwrap(),
                },
            )
        })
        .collect()
}

#[test]
fn lazy_png_crops_in_source_coordinates_on_all_four_edges_and_scales_both_axes() {
    let pixels = RgbaImage::from_fn(8, 8, |x, y| {
        Rgba(match (x < 4, y < 4) {
            (true, true) => [255, 0, 0, 255],
            (false, true) => [0, 255, 0, 255],
            (true, false) => [0, 0, 255, 255],
            (false, false) => [255, 255, 0, 255],
        })
    });
    let layout = layout(pixels);
    let original = source();
    // The 8x8 PNG occupies global source [8,6]..[24,22], not crop coordinates.
    // This 12x12 crop removes 2 source pixels from all four drawing edges.
    let area = region(
        vec![rich(&layout, [(8., 6.), (24., 22.)])],
        10.,
        8.,
        12.,
        12.,
    );
    let output = crop_region_with_rasters(&original, &area, None, &map(&[&layout])).unwrap();
    assert_eq!(output.dimensions(), (12, 12));
    for (x, y, expected) in [
        (0, 0, [255, 0, 0, 255]),
        (11, 0, [0, 255, 0, 255]),
        (0, 11, [0, 0, 255, 255]),
        (11, 11, [255, 255, 0, 255]),
    ] {
        assert_eq!(output.get_pixel(x, y), Rgba(expected));
    }
    // A second export shrinks the same immutable PNG to 4x4 source pixels.
    let area = region(
        vec![rich(&layout, [(12., 10.), (16., 14.)])],
        10.,
        8.,
        8.,
        8.,
    );
    let output = crop_region_with_rasters(&original, &area, None, &map(&[&layout])).unwrap();
    assert_eq!(output.get_pixel(2, 2), Rgba([255, 0, 0, 255]));
    assert_eq!(output.get_pixel(5, 2), Rgba([0, 255, 0, 255]));
    assert_eq!(output.get_pixel(2, 5), Rgba([0, 0, 255, 255]));
    assert_eq!(output.get_pixel(5, 5), Rgba([255, 255, 0, 255]));
    assert_eq!(output.get_pixel(1, 2), Rgba([20, 40, 100, 255]));
    assert_eq!(output.get_pixel(6, 5), Rgba([20, 40, 100, 255]));
    assert_eq!(original.get_pixel(12, 10), Rgba([20, 40, 100, 255]));
}

#[test]
fn lazy_real_png_straight_alpha_and_transparent_rgb_do_not_create_dark_or_blue_fringes() {
    let layout = layout(RgbaImage::from_fn(8, 8, |x, y| {
        if (2..6).contains(&x) && (2..6).contains(&y) {
            Rgba([240, 80, 40, 128])
        } else {
            Rgba([0, 0, 255, 0])
        }
    }));
    let area = region(
        vec![rich(&layout, [(8., 6.), (24., 22.)])],
        4.,
        2.,
        24.,
        20.,
    );
    let output = crop_region_with_rasters(&source(), &area, None, &map(&[&layout])).unwrap();
    let interior = output.get_pixel(10, 10);
    // Independent source-over arithmetic gives approx [130,60,70,255]; allow
    // one byte for unavoidable 8-bit premultiply/demultiply rounding.
    assert!((129..=131).contains(&interior[0]));
    assert!((59..=61).contains(&interior[1]));
    assert!((69..=71).contains(&interior[2]));
    assert_eq!(interior[3], 255);
    assert_eq!(output.get_pixel(4, 4), Rgba([20, 40, 100, 255]));
    assert_eq!(output.get_pixel(0, 0), Rgba([20, 40, 100, 255]));
    for pixel in output.to_rgba8().pixels() {
        assert!(pixel[0] >= 20 && pixel[1] >= 40 && pixel[2] <= 100);
        assert_eq!(pixel[3], 255);
    }
    let underlay = RgbaImage::from_pixel(24, 20, Rgba([255, 255, 255, 255]));
    let white =
        crop_region_with_rasters(&source(), &area, Some(&underlay), &map(&[&layout])).unwrap();
    let interior = white.get_pixel(10, 10);
    assert!((246..=249).contains(&interior[0]));
    assert!((166..=169).contains(&interior[1]));
    assert!((146..=149).contains(&interior[2]));
}

#[test]
fn lazy_reader_preserves_multiple_png_vector_segments_in_document_z_order() {
    let green = layout(RgbaImage::from_pixel(8, 8, Rgba([0, 200, 0, 255])));
    let orange = layout(RgbaImage::from_pixel(4, 4, Rgba([240, 140, 0, 255])));
    let area = region(
        vec![
            line("#ff0000", 12.),
            rich(&green, [(8., 6.), (24., 22.)]),
            line("#0000ff", 16.),
            rich(&orange, [(12., 10.), (20., 18.)]),
            line("#ffffff", 20.),
        ],
        4.,
        2.,
        24.,
        20.,
    );
    let mut seen = Vec::new();
    let output = crop_region_with_layouts(&source(), &area, None, |reference| {
        seen.push(reference.clone());
        if reference == green.reference() {
            decode(&green)
        } else if reference == orange.reference() {
            decode(&orange)
        } else {
            panic!("unknown reference")
        }
    })
    .unwrap();
    assert_eq!(
        seen,
        vec![green.reference().clone(), orange.reference().clone()]
    );
    assert_eq!(output.get_pixel(1, 10), Rgba([255, 0, 0, 255])); // Earlier red outside PNG.
    assert_eq!(output.get_pixel(6, 10), Rgba([0, 200, 0, 255])); // First PNG covers earlier red.
    assert_eq!(output.get_pixel(6, 14), Rgba([0, 0, 255, 255])); // Middle blue covers first PNG.
    assert_eq!(output.get_pixel(10, 14), Rgba([240, 140, 0, 255])); // Second PNG covers middle blue.
    assert_eq!(output.get_pixel(10, 18), Rgba([255, 255, 255, 255])); // Last vector covers first PNG.
}

#[test]
fn map_adapter_requires_the_complete_layout_reference_and_exact_decoded_dimensions() {
    let layout = layout(RgbaImage::from_pixel(4, 4, Rgba([200, 100, 0, 255])));
    let rasters = map(&[&layout]);
    let drawing = rich(&layout, [(8., 6.), (16., 14.)]);
    assert!(crop_region_with_rasters(
        &source(),
        &region(vec![drawing.clone()], 4., 2., 24., 20.),
        None,
        &rasters
    )
    .is_ok());
    for variant in 0..6 {
        let mut bad = drawing.clone();
        let reference = bad.rich.as_mut().unwrap();
        match variant {
            0 => reference.layout_id = uuid::Uuid::new_v4().to_string(),
            1 => reference.layout_sha256 = "c".repeat(64),
            2 => reference.raster_sha256 = "d".repeat(64),
            3 => reference.kind = RichKind::Table,
            4 => {
                reference.width = 8;
                reference.height = 8;
            }
            _ => {
                reference.width = 2;
                reference.height = 2;
            }
        }
        assert!(crop_region_with_rasters(
            &source(),
            &region(vec![bad], 4., 2., 24., 20.),
            None,
            &rasters
        )
        .is_err());
    }
    // Even an exact metadata identity cannot excuse a decoder returning wrong dimensions.
    assert!(crop_region_with_layouts(
        &source(),
        &region(vec![drawing], 4., 2., 24., 20.),
        None,
        |_| Ok(RgbaImage::new(3, 4))
    )
    .is_err());
}

#[test]
fn ordinary_vectors_never_load_a_layout_and_keep_existing_source_crop_pixels() {
    let area = region(
        vec![line("#ff0000", 12.), line("#0000ff", 16.)],
        4.,
        2.,
        24.,
        20.,
    );
    let output = crop_region_with_layouts(&source(), &area, None, |_| {
        panic!("ordinary export must not read rich DB")
    })
    .unwrap();
    assert_eq!(output.get_pixel(10, 10), Rgba([255, 0, 0, 255]));
    assert_eq!(output.get_pixel(10, 14), Rgba([0, 0, 255, 255]));
    assert_eq!(output.get_pixel(10, 6), Rgba([20, 40, 100, 255]));
}
