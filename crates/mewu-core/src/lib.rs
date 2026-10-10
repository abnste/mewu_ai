// SPDX-License-Identifier: MPL-2.0
mod connections;
mod drawing;
mod external_memory;
mod external_memory_types;
mod geometry_history;
mod journal;
mod journal_types;
mod memory;
mod model;
mod ocr;
mod rich_annotations;
mod store;
mod translation;
mod video_annotation_algorithm;
mod video_annotation_layouts;
mod video_annotations;
mod video_edit;
mod video_registered_layouts;
mod video_vector_layouts;
mod visual_annotations;
pub use connections::validate_connection_profile;
pub use external_memory_types::*;
pub use journal_types::*;
pub use model::*;
pub use ocr::validate_ocr_document;
pub use rich_annotations::{
    RasterRole, RichAlignment, RichContent, RichDrawingRef, RichKind, RichLayout, RichRendererIdentity,
    VerifiedRichLayout, MAX_RICH_CONTENT_BYTES, MAX_RICH_PIXELS, MAX_RICH_PNG_BYTES,
    MAX_RICH_RUN_CONTENT_BYTES, MAX_RICH_RUN_PNG_BYTES, RICH_DRAWING_COLOR,
    RICH_DRAWING_STROKE_WIDTH,
};
pub use store::{CoreError, ExportView, Store};
pub use translation::validate_translation_document;
pub use video_annotation_algorithm::{
    active_at as video_annotations_at, project_interval as project_video_annotation_interval,
    resolve_model_interval as resolve_video_model_interval,
    source_point as video_annotation_source_point, source_rect as video_annotation_source_rect,
};
pub use video_annotation_layouts::{VerifiedVideoTextLayout, VideoTextContent, VideoTextLayoutRef};
pub use video_annotations::*;
pub use video_registered_layouts::{
    VideoAnnotationLayouts, MAX_VIDEO_DOCUMENT_LAYOUT_DESCRIPTOR_BYTES,
    MAX_VIDEO_DOCUMENT_LAYOUT_PIXELS, MAX_VIDEO_DOCUMENT_LAYOUT_PNG_BYTES,
};
pub use video_vector_layouts::{
    VerifiedVideoVectorLayout, VideoVectorContent, VideoVectorKind, VideoVectorLayoutRef,
    MAX_VIDEO_VECTOR_DESCRIPTOR_BYTES, MAX_VIDEO_VECTOR_OUTSET, MAX_VIDEO_VECTOR_PNG_BYTES,
    MAX_VIDEO_VECTOR_POINTS,
};
pub use visual_annotations::{
    visual_source_fence, VerifiedVisualAuthority, VerifiedVisualInput, VisualAnnotationGrant,
    VisualAppendBatch, VisualAppendGroup, VisualDrawingOrigin, VisualInputManifest,
    VisualInputTarget, VisualPixelRect, VisualPixelSize, VisualSourceFence, MAX_VISUAL_OBJECTS,
    VISUAL_ANNOTATION_TOOL,
};
