// SPDX-License-Identifier: MPL-2.0
//! Bounded, non-destructive manual drawing documents. Renderers clip these
//! background-pixel objects to the current region without rewriting geometry.
use crate::{
    Asset, AssetKind, CoreError, Drawing, DrawingBatchEdit, DrawingEdit, DrawingHistory,
    DrawingKind, DrawingStep, Region,
};
use std::collections::HashSet;

type Result<T> = std::result::Result<T, CoreError>;
const MAX_OBJECTS: usize = 256;
const MAX_POINTS: usize = 4096;
const MAX_ACTIONS: usize = 128;
const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_HISTORY_BYTES: usize = 4 * 1024 * 1024;

pub(crate) enum Mutation {
    Add(Drawing),
    AppendBatch(Vec<Drawing>),
    Update(Drawing),
    UpdateBatch(Vec<Drawing>),
    Remove(String),
    Undo,
    Redo,
}

fn invalid(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}

pub(crate) fn next_revision(region: &Region) -> Result<u64> {
    region
        .drawing_revision
        .checked_add(1)
        .ok_or_else(|| invalid("绘制修订号超出范围"))
}

fn validate_object(object: &Drawing, width: f64, height: f64) -> Result<()> {
    if let Some(origin) = &object.origin {
        crate::visual_annotations::validate_origin(origin)?;
    }
    if uuid::Uuid::parse_str(&object.id).is_err() {
        return Err(invalid("绘制对象标识无效"));
    }
    if object.kind != DrawingKind::Rich && object.rich.is_some() {
        return Err(invalid("普通绘制不能携带富标注布局"));
    }
    let color = object.color.as_bytes();
    if color.len() != 7 || color[0] != b'#' || !color[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(invalid("绘制颜色必须为 #RRGGBB"));
    }
    if !object.stroke_width.is_finite() || !(0.5..=64.).contains(&object.stroke_width) {
        return Err(invalid("绘制线宽需为 0.5 至 64 像素"));
    }
    let count = object.points.len();
    let valid_count = match object.kind {
        DrawingKind::Pen | DrawingKind::Highlighter => (1..=MAX_POINTS).contains(&count),
        DrawingKind::Text | DrawingKind::Number => count == 1,
        _ => count == 2,
    };
    let floating = object
        .rich
        .as_ref()
        .is_some_and(|r| r.kind == crate::RichKind::Extracted || r.parent_id.is_some());
    if !valid_count
        || object.points.iter().any(|p| {
            !p.x.is_finite()
                || !p.y.is_finite()
                || p.x < if floating { -width * 16. } else { 0. }
                || p.y < if floating { -height * 16. } else { 0. }
                || p.x > if floating { width * 16. } else { width }
                || p.y > if floating { height * 16. } else { height }
        })
    {
        return Err(invalid("绘制点数或背景像素坐标无效"));
    }
    match object.kind {
        DrawingKind::Rich => {
            let reference = object
                .rich
                .as_ref()
                .ok_or_else(|| invalid("富标注缺少布局引用"))?;
            crate::rich_annotations::validate_ref_shape(reference)?;
            let a = object.points[0];
            let b = object.points[1];
            let w = b.x - a.x;
            let h = b.y - a.y;
            let first = w * f64::from(reference.height);
            let second = h * f64::from(reference.width);
            if (reference.kind.is_raster_edit() == object.origin.is_some())
                || object.text.is_some()
                || object.font_size.is_some()
                || object.color != crate::RICH_DRAWING_COLOR
                || object.stroke_width != crate::RICH_DRAWING_STROKE_WIDTH
                || w <= 0.
                || h <= 0.
                || (first - second).abs() > first.abs().max(second.abs()) * 1e-6
                || (reference.kind == crate::RichKind::Repair
                    && reference.parent_id.is_none()
                    && ((w - f64::from(reference.width)).abs() > 1e-6
                        || (h - f64::from(reference.height)).abs() > 1e-6))
            {
                return Err(invalid("富标注需保持来源、固定样式及等比正向矩形"));
            }
        }
        DrawingKind::Number => {
            let text = object.text.as_deref().unwrap_or_default();
            if text.is_empty()
                || text.len() > 4
                || text.starts_with('0')
                || !text.bytes().all(|c| c.is_ascii_digit())
                || object
                    .font_size
                    .is_none_or(|size| !size.is_finite() || !(22. ..=64.).contains(&size))
            {
                return Err(invalid("序号需为 1 至 9999，直径需为 22 至 64 像素"));
            }
        }
        DrawingKind::Text => {
            let text = object
                .text
                .as_deref()
                .ok_or_else(|| invalid("文字对象缺少文本"))?;
            if text.trim().is_empty()
                || text.chars().count() > 2000
                || text.chars().any(|c| c.is_control() && c != '\n')
            {
                return Err(invalid("绘制文字需为 1 至 2000 字，不能包含控制字符"));
            }
            let size = object
                .font_size
                .ok_or_else(|| invalid("文字对象缺少字号"))?;
            if !size.is_finite() || !(6. ..=256.).contains(&size) {
                return Err(invalid("绘制字号需为 6 至 256 像素"));
            }
        }
        _ if object.text.is_some() || object.font_size.is_some() => {
            return Err(invalid("非文字对象不能携带文本或字号"))
        }
        DrawingKind::Line | DrawingKind::Arrow if object.points[0] == object.points[1] => {
            return Err(invalid("直线或箭头不能只有一个位置"))
        }
        DrawingKind::Rect | DrawingKind::Ellipse | DrawingKind::Mosaic
            if object.points[0].x == object.points[1].x
                || object.points[0].y == object.points[1].y =>
        {
            return Err(invalid("图形宽高必须大于零"))
        }
        _ => {}
    }
    if object.kind == DrawingKind::Mosaic
        && (!(6. ..=40.).contains(&object.stroke_width) || object.stroke_width.fract() != 0.)
    {
        return Err(invalid("马赛克块大小需为 6 至 40 的整数"));
    }
    if object.kind == DrawingKind::Mosaic
        && (width > 16384. || height > 16384. || width * height > 32. * 1024. * 1024.)
    {
        return Err(invalid(
            "马赛克背景最多支持 3200 万像素，单边不超过 16384 像素",
        ));
    }
    Ok(())
}

fn validate_objects(objects: &[Drawing], width: f64, height: f64) -> Result<()> {
    if objects.len() > MAX_OBJECTS || serde_json::to_vec(objects)?.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("此选区的绘制内容过多"));
    }
    let mut ids = HashSet::new();
    for object in objects {
        if !ids.insert(&object.id) {
            return Err(invalid("绘制对象标识重复"));
        }
        validate_object(object, width, height)?;
        if let Some(id) = object.rich.as_ref().and_then(|r| r.parent_id.as_ref()) {
            if !objects.iter().any(|p| {
                &p.id == id
                    && p.rich
                        .as_ref()
                        .is_some_and(|r| r.kind == crate::RichKind::Extracted)
            }) {
                return Err(invalid("修补图层缺少原图"));
            }
        }
    }
    Ok(())
}

fn apply_edit(objects: &mut Vec<Drawing>, edit: &DrawingEdit, forward: bool) -> Result<()> {
    let (before, after) = if forward {
        (&edit.before, &edit.after)
    } else {
        (&edit.after, &edit.before)
    };
    match (before, after) {
        (None, Some(value))
            if edit.index <= objects.len() && !objects.iter().any(|o| o.id == value.id) =>
        {
            objects.insert(edit.index, value.clone());
        }
        (Some(expected), None) if objects.get(edit.index) == Some(expected) => {
            objects.remove(edit.index);
        }
        (Some(expected), Some(value))
            if expected.id == value.id && objects.get(edit.index) == Some(expected) =>
        {
            objects[edit.index] = value.clone();
        }
        _ => return Err(invalid("绘制历史与当前对象不一致")),
    }
    Ok(())
}

fn apply_step(objects: &mut Vec<Drawing>, step: &DrawingStep, forward: bool) -> Result<()> {
    if forward {
        for edit in step.edits() {
            apply_edit(objects, edit, true)?;
        }
    } else {
        for edit in step.edits().iter().rev() {
            apply_edit(objects, edit, false)?;
        }
    }
    Ok(())
}

pub(crate) fn validate(region: &Region, background: Option<&Asset>) -> Result<()> {
    if region.drawings.is_empty() && region.drawing_history.is_empty() {
        return Ok(());
    }
    let background = region
        .image_override
        .as_ref()
        .or(background)
        .filter(|b| b.kind == AssetKind::Image)
        .ok_or_else(|| invalid("绘制缺少图片背景"))?;
    let width = background
        .width
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid("绘制背景尺寸缺失"))? as f64;
    let height = background
        .height
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid("绘制背景尺寸缺失"))? as f64;
    if region.drawing_revision == 0 {
        return Err(invalid("绘制修订号无效"));
    }
    validate_objects(&region.drawings, width, height)?;
    let history = &region.drawing_history;
    if history.undo.len() + history.redo.len() > MAX_ACTIONS
        || serde_json::to_vec(history)?.len() > MAX_HISTORY_BYTES
    {
        return Err(invalid("绘制历史超过保留范围"));
    }
    for step in history.undo.iter().chain(&history.redo) {
        if step.edits().is_empty() || step.edits().len() > MAX_OBJECTS {
            return Err(invalid("绘制批次无效"));
        }
        for edit in step.edits() {
            if edit.index >= MAX_OBJECTS || edit.before == edit.after {
                return Err(invalid("绘制历史无效"));
            }
            for object in edit.before.iter().chain(&edit.after) {
                validate_object(object, width, height)?;
            }
        }
    }
    // Persisted histories are data too: reject forged/misaligned operations at
    // open, rather than silently losing edits when the user later presses Undo.
    for (steps, forward) in [(&history.undo, false), (&history.redo, true)] {
        let mut replay = region.drawings.clone();
        for edit in steps.iter().rev() {
            apply_step(&mut replay, edit, forward)?;
            if replay.len() > MAX_OBJECTS {
                return Err(invalid("绘制历史对象过多"));
            }
        }
    }
    Ok(())
}

fn trim_history(history: &mut DrawingHistory) -> Result<()> {
    while history.undo.len() > MAX_ACTIONS {
        history.undo.remove(0);
    }
    while serde_json::to_vec(history)?.len() > MAX_HISTORY_BYTES && history.undo.len() > 1 {
        history.undo.remove(0);
    }
    Ok(())
}

pub(crate) fn apply(region: &mut Region, background: &Asset, mutation: Mutation) -> Result<bool> {
    if let Mutation::UpdateBatch(values) = &mutation {
        return update_batch(region, background, values.clone(), true);
    }
    if let Mutation::Update(value) = &mutation {
        if region
            .drawings
            .iter()
            .any(|d| d.rich.as_ref().and_then(|r| r.parent_id.as_ref()) == Some(&value.id))
        {
            return update_batch(region, background, vec![value.clone()], false);
        }
    }
    if let Mutation::Remove(id) = &mutation {
        if region
            .drawings
            .iter()
            .any(|d| d.rich.as_ref().and_then(|r| r.parent_id.as_ref()) == Some(id))
        {
            let batch = region
                .drawings
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, d)| {
                    &d.id == id || d.rich.as_ref().and_then(|r| r.parent_id.as_ref()) == Some(id)
                })
                .map(|(index, d)| DrawingEdit {
                    index,
                    before: Some(d.clone()),
                    after: None,
                })
                .collect();
            return commit_batch(region, background, batch);
        }
    }
    if let Mutation::AppendBatch(values) = mutation {
        if values.is_empty() || values.len() > 48 {
            return Err(invalid("绘制批次无效"));
        }
        let start = region.drawings.len();
        let step = DrawingStep::Batch(DrawingBatchEdit {
            batch: values
                .into_iter()
                .enumerate()
                .map(|(offset, value)| DrawingEdit {
                    index: start + offset,
                    before: None,
                    after: Some(value),
                })
                .collect(),
        });
        apply_step(&mut region.drawings, &step, true)?;
        region.drawing_history.redo.clear();
        region.drawing_history.undo.push(step);
        trim_history(&mut region.drawing_history)?;
        region.drawing_revision = next_revision(region)?;
        validate(region, Some(background))?;
        return Ok(true);
    }
    let edit = match mutation {
        Mutation::Add(value) => {
            if value.origin.is_some() || value.kind == DrawingKind::Rich || value.rich.is_some() {
                return Err(invalid("手绘不能指定 AI 来源"));
            }
            if region.drawings.iter().any(|o| o.id == value.id) {
                return Err(invalid("绘制对象标识重复"));
            }
            DrawingEdit {
                index: region.drawings.len(),
                before: None,
                after: Some(value),
            }
        }
        Mutation::Update(value) => {
            let index = region
                .drawings
                .iter()
                .position(|o| o.id == value.id)
                .ok_or_else(|| invalid("绘制对象已不存在"))?;
            if region.drawings[index]
                .rich
                .as_ref()
                .is_some_and(|r| r.kind == crate::RichKind::Repair)
            {
                return Err(invalid("背景修补只能随历史撤销，不能移动"));
            }
            if region.drawings[index].origin != value.origin {
                return Err(invalid("不能更改绘制来源"));
            }
            if region.drawings[index].kind == DrawingKind::Rich || value.kind == DrawingKind::Rich {
                let mut geometry_only = region.drawings[index].clone();
                geometry_only.points = value.points.clone();
                if geometry_only != value {
                    return Err(invalid("富标注只能修改几何位置，不能替换布局或样式"));
                }
            }
            if region.drawings[index] == value {
                return Ok(false);
            }
            DrawingEdit {
                index,
                before: Some(region.drawings[index].clone()),
                after: Some(value),
            }
        }
        Mutation::Remove(id) => {
            let index = region
                .drawings
                .iter()
                .position(|o| o.id == id)
                .ok_or_else(|| invalid("绘制对象已不存在"))?;
            if region.drawings[index]
                .rich
                .as_ref()
                .is_some_and(|r| r.kind == crate::RichKind::Repair)
            {
                return Err(invalid("背景修补只能随历史撤销"));
            }
            DrawingEdit {
                index,
                before: Some(region.drawings[index].clone()),
                after: None,
            }
        }
        Mutation::Undo | Mutation::Redo => {
            let redo = matches!(mutation, Mutation::Redo);
            let edit = if redo {
                region.drawing_history.redo.pop()
            } else {
                region.drawing_history.undo.pop()
            };
            let Some(edit) = edit else {
                return Ok(false);
            };
            apply_step(&mut region.drawings, &edit, redo)?;
            if redo {
                region.drawing_history.undo.push(edit);
            } else {
                region.drawing_history.redo.push(edit);
            }
            region.drawing_revision = next_revision(region)?;
            validate(region, Some(background))?;
            return Ok(true);
        }
        Mutation::AppendBatch(_) | Mutation::UpdateBatch(_) => unreachable!("handled above"),
    };
    apply_edit(&mut region.drawings, &edit, true)?;
    region.drawing_history.redo.clear();
    region.drawing_history.undo.push(edit.into());
    trim_history(&mut region.drawing_history)?;
    region.drawing_revision = next_revision(region)?;
    validate(region, Some(background))?;
    Ok(true)
}

fn commit_batch(region: &mut Region, background: &Asset, batch: Vec<DrawingEdit>) -> Result<bool> {
    if batch.is_empty() {
        return Ok(false);
    }
    let step = DrawingStep::Batch(DrawingBatchEdit { batch });
    apply_step(&mut region.drawings, &step, true)?;
    region.drawing_history.redo.clear();
    region.drawing_history.undo.push(step);
    trim_history(&mut region.drawing_history)?;
    region.drawing_revision = next_revision(region)?;
    validate(region, Some(background))?;
    Ok(true)
}
fn update_batch(
    region: &mut Region,
    background: &Asset,
    values: Vec<Drawing>,
    geometry_only: bool,
) -> Result<bool> {
    if values.is_empty() || values.len() > MAX_OBJECTS {
        return Err(invalid("绘制批次无效"));
    }
    let mut ids = HashSet::new();
    let mut batch = Vec::new();
    for value in &values {
        if !ids.insert(&value.id) {
            return Err(invalid("绘制对象标识重复"));
        }
        let (index, old) = region
            .drawings
            .iter()
            .enumerate()
            .find(|(_, d)| d.id == value.id)
            .ok_or_else(|| invalid("绘制对象已不存在"))?;
        if old
            .rich
            .as_ref()
            .is_some_and(|r| r.kind == crate::RichKind::Repair)
        {
            return Err(invalid("背景修补只能随原图移动"));
        }
        if old.origin != value.origin {
            return Err(invalid("不能更改绘制来源"));
        }
        if geometry_only || old.kind == DrawingKind::Rich || value.kind == DrawingKind::Rich {
            let mut expected = old.clone();
            expected.points = value.points.clone();
            if expected != *value {
                return Err(invalid("批量移动只能修改几何位置"));
            }
        }
        if old == value {
            continue;
        }
        batch.push(DrawingEdit {
            index,
            before: Some(old.clone()),
            after: Some(value.clone()),
        });
        if old
            .rich
            .as_ref()
            .is_some_and(|r| r.kind == crate::RichKind::Extracted)
        {
            if value.points.len() != 2 {
                return Err(invalid("原图几何无效"));
            }
            let a = old.points[0];
            let b = old.points[1];
            let next = value.points[0];
            if b.x <= a.x || b.y <= a.y {
                return Err(invalid("原图几何无效"));
            }
            let sx = (value.points[1].x - next.x) / (b.x - a.x);
            let sy = (value.points[1].y - next.y) / (b.y - a.y);
            for (index, child) in region.drawings.iter().enumerate().filter(|(_, d)| {
                d.rich.as_ref().and_then(|r| r.parent_id.as_ref()) == Some(&old.id)
            }) {
                let mut after = child.clone();
                for p in &mut after.points {
                    p.x = next.x + (p.x - a.x) * sx;
                    p.y = next.y + (p.y - a.y) * sy;
                }
                batch.push(DrawingEdit {
                    index,
                    before: Some(child.clone()),
                    after: Some(after),
                });
            }
        }
    }
    batch.sort_by_key(|e| e.index);
    commit_batch(region, background, batch)
}
