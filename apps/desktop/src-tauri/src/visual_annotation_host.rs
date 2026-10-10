// SPDX-License-Identifier: MPL-2.0
//! One authorized run may optionally append editable objects to its sent images.
//! No model supplied scene IDs, filesystem paths or drawing provenance enter.
use crate::visual_annotation_input::VisualCoordinateProfile;
use crate::{ai, journal_runtime, publish, Host};
use mewu_core::{
    DispatchLease, Drawing, DrawingKind, DrawingPoint, RichAlignment, RichContent,
    RichRendererIdentity, ToolBinding, VerifiedRichLayout, VerifiedVisualAuthority,
    VerifiedVisualInput, VisualAnnotationGrant, VisualAppendBatch, VisualAppendGroup,
    MAX_VISUAL_OBJECTS, VISUAL_ANNOTATION_TOOL,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::sync::Semaphore;

pub(crate) static TABLE_WORKERS: Semaphore = Semaphore::const_new(2);
/// A dropped async invoke does not release the blocking renderer's permit.
/// Exit must wait for this observation as well as the formula child registry.
pub fn table_workers_are_idle() -> bool {
    TABLE_WORKERS.available_permits() == 2
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Image,
    Video,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOrigin {
    pub scene_id: String,
    pub grant: VisualAnnotationGrant,
    pub manifest_sha256: String,
    pub source_kind: SourceKind,
}

pub struct RunScope {
    pub origin: RunOrigin,
    pub input: VerifiedVisualInput,
    authority: VerifiedVisualAuthority,
}

impl RunScope {
    pub fn new(
        scene_id: String,
        grant: VisualAnnotationGrant,
        input: VerifiedVisualInput,
    ) -> Result<Self, String> {
        for target in &input.manifest().targets {
            VisualCoordinateProfile::from_id(&target.profile_id)?;
        }
        let authority = VerifiedVisualAuthority::new(grant.clone(), input.sha256().into())
            .map_err(journal_runtime::store_error)?;
        Ok(Self {
            origin: RunOrigin {
                scene_id,
                grant,
                manifest_sha256: input.sha256().into(),
                source_kind: SourceKind::Image,
            },
            input,
            authority,
        })
    }
    pub fn binding(&self) -> ToolBinding {
        self.origin.grant.binding(&self.origin.manifest_sha256)
    }
    pub fn definition(&self) -> Value {
        let groups: Vec<_> = self
            .input
            .manifest()
            .targets
            .iter()
            .map(group_definition)
            .collect();
        json!({"type":"function","function":{
            "name":VISUAL_ANNOTATION_TOOL,
            "description":"Optional screenshot presentation: write editable, undoable answers, annotations or drawings on the attached screenshot. Decide from the user's intent and task whether conversational text, formulas, tables or spatial presentation best helps; do not ask the user to select a send mode. Do not call merely because this tool is available, and never add annotations when the user requests a text-only reply. Use only the supplied target handles and exact coordinate contract. For normalized1000, (0,0) is top left and (1000,1000) bottom right independently on each axis; strokeWidth and fontSize always use encoded-image pixels. Text needs one point, text and fontSize; topLeft is its layout anchor, first baseline is anchor.y+mappedFontSize and line advance 1.25*mappedFontSize. Plain text never interprets math or tables. Use kind=table with versioned literal cells for a real table, or kind=formula with tex, color and fontSize for rendered math. Table/formula need two ordered top-left/bottom-right points defining available bounds; complete content fits proportionally or the whole call fails if unreadable. They must omit strokeWidth/text/anchor and cannot carry images, SVG, paths or layout references. Arrow/line need different endpoints; rect/ellipse need opposite corners. If using this tool, call once with all intended objects when possible.",
            "parameters":{"type":"object","additionalProperties":false,"required":["groups"],"properties":{
                "groups":{"type":"array","minItems":1,"maxItems":6,"items":{"oneOf":groups}}
            }}
        }})
    }

    pub fn live(&self, engine: &crate::Engine, scene: &str, run: &str) -> bool {
        self.origin.scene_id == scene
            && engine.visual_runs.get(run) == Some(&self.origin)
            && crate::run_is_authorized(engine, scene, run, None)
    }

    pub async fn execute(
        &self,
        app: &AppHandle,
        scene: &str,
        run: &str,
        call: ai::ModelToolCall,
        lease: DispatchLease,
    ) -> Result<journal_runtime::CommittedToolOutput, String> {
        if call.name != VISUAL_ANNOTATION_TOOL {
            return self.not_sent(
                app,
                scene,
                run,
                &lease,
                mewu_core::NotSentReason::NotAdvertised,
                "本次运行未启用该工具".into(),
            );
        }
        let plan = match parse_plan(&call.arguments, &self.input) {
            Ok(plan) => plan,
            Err(error) => {
                self.check_live(app, scene, run)?;
                let host = app.state::<Host>();
                let mut engine = host.lock()?;
                let receipt = engine
                    .store
                    .record_tool_not_sent(&lease, mewu_core::NotSentReason::InvalidArguments)
                    .map_err(journal_runtime::store_error)?;
                journal_runtime::emit_changed(app, scene, run, receipt.journal_revision);
                publish(app, &engine.store.snapshot());
                // No drawing was dispatched. Let the existing bounded tool loop
                // consume an explicit error and correct its next call; never
                // turn a malformed call into a successful execution receipt.
                return Ok(journal_runtime::CommittedToolOutput {model_json:json!({"error":error,"executed":false,"correction":"Use the advertised visual_annotate JSON schema exactly. Keep the supplied target, coordinateSpace and styleUnit. Text: one point, text, fontSize, color and anchor=topLeft. Shapes: points, color and strokeWidth. Tables/formulas: their own fields only, no strokeWidth or text. Correct the parameters or explain the failure; do not claim annotations were drawn."}).to_string()});
            }
        };
        if let Err(error) = self.check_live(app, scene, run) {
            return self.not_sent(
                app,
                scene,
                run,
                &lease,
                mewu_core::NotSentReason::AuthorityChanged,
                error,
            );
        }
        let root = app.state::<Host>().root.clone();
        let (batch, layouts) =
            match render_plan(plan, root, || self.check_live(app, scene, run)).await {
                Ok(result) => result,
                Err(error) => {
                    return self.not_sent(
                        app,
                        scene,
                        run,
                        &lease,
                        mewu_core::NotSentReason::PreparationFailed,
                        error,
                    )
                }
            };
        let result = self.commit(app, scene, run, &lease, batch, layouts);
        match result {
            Ok(value) => Ok(value),
            Err(error) => self.not_sent(
                app,
                scene,
                run,
                &lease,
                mewu_core::NotSentReason::AuthorityChanged,
                error,
            ),
        }
    }

    fn not_sent(
        &self,
        app: &AppHandle,
        scene: &str,
        run: &str,
        lease: &DispatchLease,
        reason: mewu_core::NotSentReason,
        error: String,
    ) -> Result<journal_runtime::CommittedToolOutput, String> {
        let host = app.state::<Host>();
        let mut engine = host.lock()?;
        // Core is idempotent for NotSent, and refuses to rewrite Responded or
        // terminated observations. No error here is called an unknown effect.
        match engine.store.record_tool_not_sent(lease, reason) {
            Ok(receipt) => {
                journal_runtime::emit_changed(app, scene, run, receipt.journal_revision);
                publish(app, &engine.store.snapshot());
                Err(error)
            }
            Err(record_error) => Err(format!(
                "{error}；执行记录未能更新：{}",
                journal_runtime::store_error(record_error)
            )),
        }
    }

    fn check_live(&self, app: &AppHandle, scene: &str, run: &str) -> Result<(), String> {
        let host = app.state::<Host>();
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins.store.visual_annotations(&self.origin.grant)?;
        let engine = host.lock()?;
        host.exit.ensure_background()?;
        if !self.live(&engine, scene, run) {
            return Err("原位作答已取消或插件已停用".into());
        }
        let persisted = engine
            .store
            .visual_input_for_run(scene, run, &self.authority)
            .map_err(journal_runtime::store_error)?;
        if persisted != self.input {
            return Err("原位作答来源已变更".into());
        }
        Ok(())
    }

    fn commit(
        &self,
        app: &AppHandle,
        scene: &str,
        run: &str,
        lease: &DispatchLease,
        batch: VisualAppendBatch,
        layouts: Vec<VerifiedRichLayout>,
    ) -> Result<journal_runtime::CommittedToolOutput, String> {
        let host = app.state::<Host>();
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins.store.visual_annotations(&self.origin.grant)?;
        let mut engine = host.lock()?;
        host.exit.ensure_background()?;
        if !self.live(&engine, scene, run) {
            return Err("原位作答已取消或插件已停用".into());
        }
        if engine
            .store
            .visual_input_for_run(scene, run, &self.authority)
            .map_err(journal_runtime::store_error)?
            != self.input
        {
            return Err("原位作答来源已变更".into());
        }
        let committed = engine
            .store
            .apply_visual_annotations_with_rich_receipt(lease, &self.authority, batch, layouts)
            .map_err(journal_runtime::store_error)?;
        journal_runtime::emit_changed(app, scene, run, committed.receipt.journal_revision);
        publish(app, &committed.snapshot);
        committed
            .result
            .model_visible_json
            .map(|model_json| journal_runtime::CommittedToolOutput { model_json })
            .ok_or_else(|| "原位作答缺少提交回执".into())
    }
}

fn group_definition(target: &mewu_core::VisualInputTarget) -> Value {
    // RunScope::new rejects unknown profiles before any definition is exposed.
    let normalized = target.profile_id == VisualCoordinateProfile::ExplicitNormalized1000V2.id();
    let scale = style_scale(target).max(1.);
    let max_stroke = (64. / scale).min(16.);
    let max_font = (256. / scale).min(72.);
    let point = json!({"type":"object","additionalProperties":false,"required":["x","y"],"properties":{
        "x":{"type":"number","minimum":0,"maximum":if normalized {1000} else {target.encoded.width}},
        "y":{"type":"number","minimum":0,"maximum":if normalized {1000} else {target.encoded.height}}
    }});
    let shape = |kinds: Value, min: usize, max: usize| {
        json!({"type":"object","additionalProperties":false,"required":["kind","color","strokeWidth","points"],"properties":{
            "kind":{"type":"string","enum":kinds},"color":{"type":"string","pattern":"^#[0-9a-fA-F]{6}$"},
            "strokeWidth":{"type":"number","minimum":0.5,"maximum":max_stroke,"description":"Encoded-image pixels, not normalized coordinates."},
            "points":{"type":"array","minItems":min,"maxItems":max,"items":point}
        }})
    };
    let mut text = json!({"type":"object","additionalProperties":false,"required":["kind","color","points","text","fontSize"],"properties":{
        "kind":{"const":"text"},"color":{"type":"string","pattern":"^#[0-9a-fA-F]{6}$"},
        "strokeWidth":{"type":"number","minimum":0.5,"maximum":max_stroke},
        "points":{"type":"array","minItems":1,"maxItems":1,"items":point},
        "text":{"type":"string","minLength":1,"maxLength":500},"fontSize":{"type":"number","minimum":8,"maximum":max_font,"description":"Encoded-image pixels, not normalized coordinates."}
    }});
    if normalized {
        text["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("anchor"));
        text["properties"]["anchor"] = json!({"const":"topLeft","description":"Nominal text layout top-left; not glyph center or baseline."});
    }
    let table = json!({"type":"object","additionalProperties":false,"required":["kind","points","table"],"properties":{
        "kind":{"const":"table"},"points":{"type":"array","minItems":2,"maxItems":2,"items":point},
        "table":{"type":"object","additionalProperties":false,"required":["version","header","rows","align"],"properties":{
            "version":{"const":1},"header":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"string"}},
            "rows":{"type":"array","maxItems":199,"items":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"string"}}},
            "align":{"type":"array","minItems":1,"maxItems":32,"items":{"enum":["left","center","right",null]}}
        }}
    }});
    let formula = json!({"type":"object","additionalProperties":false,"required":["kind","points","tex","color","fontSize"],"properties":{
        "kind":{"const":"formula"},"points":{"type":"array","minItems":2,"maxItems":2,"items":point},
        "tex":{"type":"string","minLength":1,"maxLength":2048},"color":{"type":"string","pattern":"^#[0-9a-fA-F]{6}$"},
        "fontSize":{"type":"number","minimum":8,"maximum":(128./scale).min(72.),"description":"Encoded-image pixels, not normalized coordinates. Complete rendered formula fits proportionally within bounds."}
    }});
    let object = json!({"oneOf":[shape(json!(["pen","highlighter"]),1,128),shape(json!(["line","arrow","rect","ellipse"]),2,2),text,table,formula]});
    let mut group = json!({"type":"object","additionalProperties":false,"required":["target","objects"],"properties":{
        "target":{"const":target.handle},
        "objects":{"type":"array","minItems":1,"maxItems":MAX_VISUAL_OBJECTS,"items":object}
    }});
    if normalized {
        group["required"]
            .as_array_mut()
            .unwrap()
            .extend([json!("coordinateSpace"), json!("styleUnit")]);
        group["properties"]["coordinateSpace"] = json!({"const":"normalized1000"});
        group["properties"]["styleUnit"] = json!({"const":"encodedPixels"});
    }
    group
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    groups: Vec<Group>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    target: String,
    #[serde(
        default,
        rename = "coordinateSpace",
        deserialize_with = "explicit_optional"
    )]
    coordinate_space: Option<CoordinateSpace>,
    #[serde(default, rename = "styleUnit", deserialize_with = "explicit_optional")]
    style_unit: Option<StyleUnit>,
    objects: Vec<Value>,
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
enum CoordinateSpace {
    Normalized1000,
}
impl TryFrom<String> for CoordinateSpace {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "normalized1000" => Ok(Self::Normalized1000),
            _ => Err("批注坐标单位无效"),
        }
    }
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
enum StyleUnit {
    EncodedPixels,
}
impl TryFrom<String> for StyleUnit {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "encodedPixels" => Ok(Self::EncodedPixels),
            _ => Err("批注样式单位无效"),
        }
    }
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
enum TextAnchor {
    TopLeft,
}
impl TryFrom<String> for TextAnchor {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "topLeft" => Ok(Self::TopLeft),
            _ => Err("批注文字锚点无效"),
        }
    }
}
// Missing is allowed only by the legacy contract. Explicit null is never a
// substitute for a required unit/anchor or an accepted legacy extension.
fn explicit_optional<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Object {
    kind: DrawingKind,
    color: String,
    stroke_width: Option<f64>,
    points: Vec<DrawingPoint>,
    text: Option<String>,
    font_size: Option<f64>,
    #[serde(default, deserialize_with = "explicit_optional")]
    anchor: Option<TextAnchor>,
}

enum ModelObject {
    Table(TableObject),
    Formula(FormulaObject),
    Vector(Object),
}
fn parse_object(value: Value) -> Result<ModelObject, String> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("缺少 kind")?;
    let fields: &[&str] = match kind {
        "table" => &["points", "table"],
        "formula" => &["points", "tex", "color", "fontSize"],
        "text" => &["points", "text", "fontSize", "color"],
        "pen" | "highlighter" | "line" | "arrow" | "rect" | "ellipse" => {
            &["points", "color", "strokeWidth"]
        }
        _ => return Err("kind 不受支持".into()),
    };
    for field in fields {
        if value.get(*field).is_none_or(Value::is_null) {
            return Err(format!("缺少 {field}"));
        }
    }
    match kind {
        "table" => serde_json::from_value(value)
            .map(ModelObject::Table)
            .map_err(|_| {
                "table 需要 version、header、rows、align；单元格为文本，不附带文字或线宽字段".into()
            }),
        "formula" => serde_json::from_value(value)
            .map(ModelObject::Formula)
            .map_err(|_| {
                "formula 需要 points、tex、color、fontSize，不附带 text、strokeWidth 或 anchor"
                    .into()
            }),
        _ => serde_json::from_value(value)
            .map(ModelObject::Vector)
            .map_err(|_| "图元字段类型无效或包含不支持的字段，points 使用 [{x,y}] 数组".into()),
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum TableTag {
    Table,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum FormulaTag {
    Formula,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableBody {
    version: u32,
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    align: Vec<Option<RichAlignment>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableObject {
    #[serde(rename = "kind")]
    _kind: TableTag,
    points: Vec<DrawingPoint>,
    table: TableBody,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FormulaObject {
    #[serde(rename = "kind")]
    _kind: FormulaTag,
    points: Vec<DrawingPoint>,
    tex: String,
    color: String,
    font_size: f64,
}

enum ParsedObject {
    Vector(Drawing),
    Rich {
        bounds: [DrawingPoint; 2],
        style_scale: f64,
        content: RichContent,
    },
}
struct ParsedGroup {
    target_handle: String,
    objects: Vec<ParsedObject>,
}
struct ParsedBatch {
    groups: Vec<ParsedGroup>,
}

fn mapped_points(
    points: Vec<DrawingPoint>,
    profile: VisualCoordinateProfile,
    target: &mewu_core::VisualInputTarget,
) -> Result<Vec<DrawingPoint>, String> {
    points
        .into_iter()
        .map(|p| {
            let encoded = profile.encoded_point(p, target.encoded)?;
            target
                .source_point(encoded)
                .map_err(journal_runtime::store_error)
        })
        .collect()
}
fn rich_bounds(
    points: Vec<DrawingPoint>,
    profile: VisualCoordinateProfile,
    target: &mewu_core::VisualInputTarget,
) -> Result<[DrawingPoint; 2], String> {
    if points.len() != 2 {
        return Err("表格或公式需要两个角点".into());
    }
    let points = mapped_points(points, profile, target)?;
    if points[1].x <= points[0].x || points[1].y <= points[0].y {
        return Err("表格或公式的可用范围无效".into());
    }
    Ok([points[0], points[1]])
}

#[cfg(test)]
fn parse_batch(arguments: &str, input: &VerifiedVisualInput) -> Result<VisualAppendBatch, String> {
    let plan = parse_plan(arguments, input)?;
    let mut groups = Vec::new();
    for group in plan.groups {
        let drawings = group
            .objects
            .into_iter()
            .map(|object| match object {
                ParsedObject::Vector(drawing) => Ok(drawing),
                ParsedObject::Rich { .. } => Err("富图层需要原生排版".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        groups.push(VisualAppendGroup {
            target_handle: group.target_handle,
            drawings,
        });
    }
    Ok(VisualAppendBatch { groups })
}

fn parse_plan(arguments: &str, input: &VerifiedVisualInput) -> Result<ParsedBatch, String> {
    if arguments.len() > 32 * 1024 {
        return Err("原位作答内容过长".into());
    }
    let args: Arguments = serde_json::from_str(arguments)
        .map_err(|_| "批注参数需要包含 groups 数组，每组包含 target、objects 和规定的坐标单位")?;
    let count = args
        .groups
        .iter()
        .try_fold(0usize, |n, g| n.checked_add(g.objects.len()))
        .ok_or("原位作答图元过多")?;
    if args.groups.is_empty() || args.groups.len() > 6 || count == 0 || count > MAX_VISUAL_OBJECTS {
        return Err("一次最多添加 48 个批注图元".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut groups = Vec::new();
    for group in args.groups {
        if group.objects.is_empty() || !seen.insert(group.target.clone()) {
            return Err("原位作答目标无效".into());
        }
        let target = input
            .manifest()
            .targets
            .iter()
            .find(|t| t.handle == group.target)
            .ok_or("原位作答目标不在本次图片中")?;
        let profile = VisualCoordinateProfile::from_id(&target.profile_id)?;
        if profile.normalized() {
            if group.coordinate_space != Some(CoordinateSpace::Normalized1000)
                || group.style_unit != Some(StyleUnit::EncodedPixels)
            {
                return Err("原位作答缺少明确坐标或样式单位".into());
            }
        } else if group.coordinate_space.is_some() || group.style_unit.is_some() {
            return Err("原位作答坐标与附件配置不匹配".into());
        }
        let scale = style_scale(target);
        let mut drawings = Vec::new();
        for (index, value) in group.objects.into_iter().enumerate() {
            let object =
                parse_object(value).map_err(|error| format!("第 {} 个批注：{error}", index + 1))?;
            let object = match object {
                ModelObject::Table(object) => {
                    let content = RichContent::Table {
                        version: object.table.version,
                        header: object.table.header,
                        rows: object.table.rows,
                        align: object.table.align,
                    };
                    crate::rich_annotation_host::table_document(&content)?;
                    drawings.push(ParsedObject::Rich {
                        bounds: rich_bounds(object.points, profile, target)?,
                        style_scale: scale,
                        content,
                    });
                    continue;
                }
                ModelObject::Formula(object) => {
                    if !object.font_size.is_finite()
                        || !(8. ..=72.).contains(&object.font_size)
                        || object.tex.trim().is_empty()
                        || object.tex.contains('\r')
                    {
                        return Err("公式字号无效".into());
                    }
                    let font_size = object.font_size * scale;
                    crate::formula_layout::FormulaInput {
                        version: 1,
                        tex: object.tex.clone(),
                        color: object.color.clone(),
                        font_size,
                    }
                    .validate()
                    .map_err(|_| "公式内容或字号无效")?;
                    let content = RichContent::Formula {
                        version: 1,
                        tex: object.tex,
                        color: object.color,
                        font_size,
                    };
                    drawings.push(ParsedObject::Rich {
                        bounds: rich_bounds(object.points, profile, target)?,
                        style_scale: scale,
                        content,
                    });
                    continue;
                }
                ModelObject::Vector(object) => object,
            };
            if profile.normalized() && object.kind == DrawingKind::Text {
                if object.anchor != Some(TextAnchor::TopLeft) {
                    return Err("原位作答文字缺少明确锚点".into());
                }
            } else if object.anchor.is_some() {
                return Err("原位作答图元锚点无效".into());
            }
            let stroke_width = object
                .stroke_width
                .or_else(|| (object.kind == DrawingKind::Text).then_some(1.))
                .ok_or("形状缺少 strokeWidth")?;
            if matches!(
                object.kind,
                DrawingKind::Mosaic | DrawingKind::Number | DrawingKind::Rich
            ) || object.points.is_empty()
                || object.points.len() > 128
                || !stroke_width.is_finite()
                || !(0.5..=16.).contains(&stroke_width)
                || object
                    .text
                    .as_ref()
                    .is_some_and(|t| t.chars().count() > 500)
                || object
                    .font_size
                    .is_some_and(|s| !s.is_finite() || !(8. ..=72.).contains(&s))
            {
                return Err("原位作答图元无效".into());
            }
            let points = mapped_points(object.points, profile, target)?;
            let drawing = Drawing {
                id: uuid::Uuid::new_v4().to_string(),
                kind: object.kind,
                color: object.color,
                stroke_width: stroke_width * scale,
                points,
                text: object.text,
                font_size: object.font_size.map(|s| s * scale),
                origin: None,
                rich: None,
            };
            validate_vector(&drawing)?;
            drawings.push(ParsedObject::Vector(drawing));
        }
        groups.push(ParsedGroup {
            target_handle: group.target,
            objects: drawings,
        });
    }
    Ok(ParsedBatch { groups })
}

fn validate_vector(drawing: &Drawing) -> Result<(), String> {
    let color = drawing.color.as_bytes();
    let count = drawing.points.len();
    let valid_count = match drawing.kind {
        DrawingKind::Pen | DrawingKind::Highlighter => (1..=128).contains(&count),
        DrawingKind::Text => count == 1,
        _ => count == 2,
    };
    if !valid_count
        || color.len() != 7
        || color[0] != b'#'
        || !color[1..].iter().all(u8::is_ascii_hexdigit)
        || !drawing.stroke_width.is_finite()
        || !(0.5..=64.).contains(&drawing.stroke_width)
    {
        return Err("原位作答图元样式或点数无效".into());
    }
    if drawing.kind == DrawingKind::Text {
        if drawing.text.as_ref().is_none_or(|text| {
            text.trim().is_empty() || text.chars().any(|c| c.is_control() && c != '\n')
        }) || drawing
            .font_size
            .is_none_or(|size| !size.is_finite() || !(6. ..=256.).contains(&size))
        {
            return Err("原位作答文字或字号无效".into());
        }
    } else if drawing.text.is_some() || drawing.font_size.is_some() {
        return Err("普通图元不能携带文字或字号".into());
    }
    if matches!(drawing.kind, DrawingKind::Line | DrawingKind::Arrow)
        && drawing.points[0] == drawing.points[1]
        || matches!(drawing.kind, DrawingKind::Rect | DrawingKind::Ellipse)
            && (drawing.points[0].x == drawing.points[1].x
                || drawing.points[0].y == drawing.points[1].y)
    {
        return Err("原位作答图元范围无效".into());
    }
    Ok(())
}

fn fit(
    bounds: [DrawingPoint; 2],
    style_scale: f64,
    layout: &VerifiedRichLayout,
) -> Result<[DrawingPoint; 2], String> {
    let width = bounds[1].x - bounds[0].x;
    let height = bounds[1].y - bounds[0].y;
    let maximum = match layout.content() {
        RichContent::Table { .. } => style_scale,
        RichContent::Formula { .. } => 1.,
        RichContent::Raster { .. } => return Err("AI 批注不能指定本地修补图层".into()),
    };
    let scale = (width / f64::from(layout.width()))
        .min(height / f64::from(layout.height()))
        .min(maximum);
    let readable = match layout.content() {
        RichContent::Table { .. } => scale / style_scale >= 0.65,
        RichContent::Formula { font_size, .. } => font_size * scale >= 8.,
        RichContent::Raster { .. } => false,
    };
    if !scale.is_finite() || scale <= 0. || !readable {
        return Err("可用范围太小，无法完整显示表格或公式".into());
    }
    Ok([
        bounds[0],
        DrawingPoint {
            x: bounds[0].x + f64::from(layout.width()) * scale,
            y: bounds[0].y + f64::from(layout.height()) * scale,
        },
    ])
}

async fn render_plan(
    plan: ParsedBatch,
    root: PathBuf,
    check: impl Fn() -> Result<(), String>,
) -> Result<(VisualAppendBatch, Vec<VerifiedRichLayout>), String> {
    let mut groups = Vec::new();
    let mut layouts = Vec::new();
    let mut content_bytes = 0_u64;
    let mut png_bytes = 0_u64;
    let mut pixels = 0_u64;
    for group in plan.groups {
        let mut drawings = Vec::new();
        for object in group.objects {
            check()?;
            match object {
                ParsedObject::Vector(drawing) => drawings.push(drawing),
                ParsedObject::Rich {
                    bounds,
                    style_scale,
                    content,
                } => {
                    let layout = match &content {
                        RichContent::Raster { .. } => {
                            return Err("AI 批注不能指定本地修补图层".into())
                        }
                        RichContent::Table { .. } => {
                            let permit = TABLE_WORKERS
                                .acquire()
                                .await
                                .map_err(|_| "表格排版不可用")?;
                            check()?;
                            let content = content.clone();
                            tauri::async_runtime::spawn_blocking(move || {
                                let _permit = permit;
                                crate::rich_annotation_host::render_table(content)
                            })
                            .await
                            .map_err(|_| "表格排版中断")??
                        }
                        RichContent::Formula {
                            version,
                            tex,
                            color,
                            font_size,
                        } => {
                            let input = crate::formula_layout::FormulaInput {
                                version: u8::try_from(*version).map_err(|_| "公式版本无效")?,
                                tex: tex.clone(),
                                color: color.clone(),
                                font_size: *font_size,
                            };
                            let result = crate::formula_layout::render_formula_embedded(
                                input.clone(),
                                root.clone(),
                                crate::formula_layout::Cancellation::default(),
                            )
                            .await?;
                            let parts = result.into_parts()?;
                            if parts.input != input {
                                return Err("公式排版内容已变更".into());
                            }
                            VerifiedRichLayout::new(
                                uuid::Uuid::new_v4().to_string(),
                                content.clone(),
                                RichRendererIdentity {
                                    id: parts.renderer_id.into(),
                                    version: format!(
                                        "{:x}",
                                        Sha256::digest(parts.style_identity.as_bytes())
                                    ),
                                    font_sha256: format!(
                                        "{:x}",
                                        Sha256::digest(parts.font_identity.as_bytes())
                                    ),
                                    style_version: 1,
                                },
                                parts.width,
                                parts.height,
                                parts.png,
                            )
                            .map_err(journal_runtime::store_error)?
                        }
                    };
                    check()?;
                    content_bytes = content_bytes
                        .checked_add(
                            serde_json::to_vec(layout.content())
                                .map_err(|_| "图层内容无效")?
                                .len() as u64,
                        )
                        .ok_or("图层过大")?;
                    png_bytes = png_bytes
                        .checked_add(layout.png().len() as u64)
                        .ok_or("图层过大")?;
                    pixels = pixels
                        .checked_add(u64::from(layout.width()) * u64::from(layout.height()))
                        .ok_or("图层过大")?;
                    if content_bytes > mewu_core::MAX_RICH_RUN_CONTENT_BYTES
                        || png_bytes > mewu_core::MAX_RICH_RUN_PNG_BYTES
                        || pixels > mewu_core::MAX_RICH_PIXELS
                    {
                        return Err("原位作答图层超过预算".into());
                    }
                    let points = fit(bounds, style_scale, &layout)?.to_vec();
                    drawings.push(Drawing {
                        id: uuid::Uuid::new_v4().to_string(),
                        kind: DrawingKind::Rich,
                        color: mewu_core::RICH_DRAWING_COLOR.into(),
                        stroke_width: mewu_core::RICH_DRAWING_STROKE_WIDTH,
                        points,
                        text: None,
                        font_size: None,
                        origin: None,
                        rich: Some(layout.reference().clone()),
                    });
                    layouts.push(layout);
                }
            }
        }
        groups.push(VisualAppendGroup {
            target_handle: group.target_handle,
            drawings,
        });
    }
    check()?;
    Ok((VisualAppendBatch { groups }, layouts))
}

fn style_scale(target: &mewu_core::VisualInputTarget) -> f64 {
    let sx = f64::from(target.tile.width) / f64::from(target.encoded.width)
        * f64::from(target.crop.width)
        / f64::from(target.resized.width);
    let sy = f64::from(target.tile.height) / f64::from(target.encoded.height)
        * f64::from(target.crop.height)
        / f64::from(target.resized.height);
    (sx * sy).sqrt()
}

#[cfg(all(test, windows))]
#[path = "visual_provider_probe_test.rs"]
mod provider_probe_tests;

#[cfg(test)]
#[path = "visual_rich_annotation_test.rs"]
mod rich_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_without_shape_line_width_is_valid_but_units_targets_and_unknown_fields_are_not_guessed()
    {
        let input = normalized_input();
        let mut value = normalized_args(&input);
        value["groups"][0]["objects"][0]
            .as_object_mut()
            .unwrap()
            .remove("strokeWidth");
        assert_eq!(
            parse_batch(&value.to_string(), &input).unwrap().groups[0].drawings[0].stroke_width,
            2.
        );
        let mut missing = value.clone();
        missing["groups"][0]["objects"][0]
            .as_object_mut()
            .unwrap()
            .remove("fontSize");
        assert_eq!(
            parse_batch(&missing.to_string(), &input).unwrap_err(),
            "第 1 个批注：缺少 fontSize"
        );
        for field in ["path", "origin", "id", "sceneId"] {
            let mut bad = value.clone();
            bad["groups"][0]["objects"][0][field] = json!("private-secret");
            let error = parse_batch(&bad.to_string(), &input).unwrap_err();
            assert!(!error.contains("private-secret"));
        }
        let mut missing = value;
        missing["groups"][0]
            .as_object_mut()
            .unwrap()
            .remove("coordinateSpace");
        assert!(parse_batch(&missing.to_string(), &input).is_err());
    }
    pub(super) fn input() -> VerifiedVisualInput {
        use mewu_core::{
            VisualInputManifest, VisualInputTarget, VisualPixelRect, VisualPixelSize,
            VisualSourceFence,
        };
        let id = || uuid::Uuid::new_v4().to_string();
        VerifiedVisualInput::new(VisualInputManifest {
            version: 1,
            targets: vec![VisualInputTarget {
                handle: id(),
                attachment_id: id(),
                attachment_ordinal: 0,
                attachment_sha256: "a".repeat(64),
                region_id: id(),
                source: VisualSourceFence {
                    background_id: id(),
                    effective_source_id: id(),
                    source_size: VisualPixelSize {
                        width: 1600,
                        height: 1200,
                    },
                    drawing_revision: 2,
                    visual_sha256: "b".repeat(64),
                },
                crop: VisualPixelRect {
                    x: 100,
                    y: 200,
                    width: 1000,
                    height: 600,
                },
                resized: VisualPixelSize {
                    width: 500,
                    height: 300,
                },
                tile: VisualPixelRect {
                    x: 100,
                    y: 50,
                    width: 300,
                    height: 200,
                },
                encoded: VisualPixelSize {
                    width: 300,
                    height: 200,
                },
                profile_id: "encoded-pixels-best-effort-v1".into(),
            }],
        })
        .unwrap()
    }
    pub(super) fn args(input: &VerifiedVisualInput) -> Value {
        json!({"groups":[{"target":input.manifest().targets[0].handle,"objects":[{"kind":"text","color":"#ff0000","strokeWidth":1,"points":[{"x":10,"y":20}],"text":"4","fontSize":16}]}]})
    }
    #[test]
    fn encoded_points_and_style_map_to_source_without_accepting_origins() {
        let input = input();
        let args = args(&input);
        let batch = parse_batch(&args.to_string(), &input).unwrap();
        let object = &batch.groups[0].drawings[0];
        assert_eq!(object.points, vec![DrawingPoint { x: 320., y: 340. }]);
        assert_eq!(object.stroke_width, 2.);
        assert_eq!(object.font_size, Some(32.));
        assert!(object.origin.is_none());
        assert!(uuid::Uuid::parse_str(&object.id).is_ok());
        for field in ["origin", "id", "sceneId", "path"] {
            let mut bad = args.clone();
            bad["groups"][0]["objects"][0][field] = json!("spoof");
            assert!(parse_batch(&bad.to_string(), &input).is_err());
        }
    }
    #[test]
    fn rejects_unknown_target_out_of_content_and_duplicate_groups() {
        let input = input();
        let original = args(&input);
        let mut bad = original.clone();
        bad["groups"][0]["target"] = json!(uuid::Uuid::new_v4().to_string());
        assert!(parse_batch(&bad.to_string(), &input).is_err());
        for coordinate in [-1., 301.] {
            let mut bad = original.clone();
            bad["groups"][0]["objects"][0]["points"][0]["x"] = json!(coordinate);
            assert!(parse_batch(&bad.to_string(), &input).is_err());
        }
        let mut bad = original.clone();
        let duplicate = bad["groups"][0].clone();
        bad["groups"].as_array_mut().unwrap().push(duplicate);
        assert!(parse_batch(&bad.to_string(), &input).is_err());
        let mut bad = original.clone();
        bad["groups"][0]["objects"][0]["kind"] = json!("mosaic");
        assert!(parse_batch(&bad.to_string(), &input).is_err());
        let mut bad = original;
        let object = bad["groups"][0]["objects"][0].clone();
        bad["groups"][0]["objects"] = json!(vec![object; 49]);
        assert!(parse_batch(&bad.to_string(), &input).is_err());
    }
    #[test]
    fn catalog_and_receipt_binding_share_the_exact_manifest_authority() {
        let input = input();
        let hash = input.sha256().to_owned();
        let handle = input.manifest().targets[0].handle.clone();
        let scope = RunScope::new(
            uuid::Uuid::new_v4().to_string(),
            VisualAnnotationGrant {
                plugin_id: "mewu.annotations".into(),
                plugin_revision: 4,
                contribution_id: "annotate".into(),
            },
            input,
        )
        .unwrap();
        let definition = scope.definition();
        assert_eq!(
            definition.pointer("/function/name").unwrap(),
            VISUAL_ANNOTATION_TOOL
        );
        assert_eq!(
            definition
                .pointer(
                    "/function/parameters/properties/groups/items/oneOf/0/properties/target/const"
                )
                .unwrap(),
            &json!(handle)
        );
        assert!(
            matches!(scope.binding(),ToolBinding::VisualAnnotation{target_manifest_sha256,plugin_revision:4,..} if target_manifest_sha256==hash)
        );
        let schema = definition.pointer("/function/parameters").unwrap();
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(schema)
            .unwrap();
        let good = args(&scope.input);
        assert!(validator.is_valid(&good));
        for field in ["text", "fontSize"] {
            let mut bad = good.clone();
            bad["groups"][0]["objects"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(!validator.is_valid(&bad));
        }
        let mut bad = good.clone();
        bad["groups"][0]["objects"][0]["kind"] = json!("number");
        assert!(!validator.is_valid(&bad));
        let mut bad = good.clone();
        bad["groups"][0]["objects"][0]["points"] = json!([{"x":10,"y":20},{"x":20,"y":30}]);
        assert!(!validator.is_valid(&bad));
    }

    pub(super) fn normalized_input() -> VerifiedVisualInput {
        let mut manifest = input().manifest().clone();
        manifest.targets[0].profile_id = VisualCoordinateProfile::ExplicitNormalized1000V2
            .id()
            .into();
        VerifiedVisualInput::new(manifest).unwrap()
    }
    pub(super) fn normalized_args(input: &VerifiedVisualInput) -> Value {
        let mut value = args(input);
        value["groups"][0]["coordinateSpace"] = json!("normalized1000");
        value["groups"][0]["styleUnit"] = json!("encodedPixels");
        value["groups"][0]["objects"][0]["anchor"] = json!("topLeft");
        value
    }
    pub(super) fn scope(input: VerifiedVisualInput) -> RunScope {
        RunScope::new(
            uuid::Uuid::new_v4().to_string(),
            VisualAnnotationGrant {
                plugin_id: "mewu.annotations".into(),
                plugin_revision: 1,
                contribution_id: "annotate".into(),
            },
            input,
        )
        .unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn screenshot_tool_can_reply_in_text_without_effects_or_a_second_request() {
        let scope = scope(normalized_input());
        let input = scope.input.clone();
        let origin = scope.origin.clone();
        ai::tests::assert_optional_annotation_text_turn(scope.definition()).await;
        assert_eq!(scope.input, input);
        assert_eq!(scope.origin, origin);
    }

    #[test]
    fn normalized_schema_and_parser_require_units_and_text_anchor_without_null_defaults() {
        let scope = scope(normalized_input());
        let good = normalized_args(&scope.input);
        let definition = scope.definition();
        let schema = definition.pointer("/function/parameters").unwrap();
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(schema)
            .unwrap();
        assert!(validator.is_valid(&good));
        let batch = parse_batch(&good.to_string(), &scope.input).unwrap();
        assert_eq!(
            batch.groups[0].drawings[0].points,
            vec![DrawingPoint { x: 306., y: 308. }]
        );
        // Position units changed, style units did not: the inverse image scale is 2.
        assert_eq!(batch.groups[0].drawings[0].font_size, Some(32.));
        assert_eq!(batch.groups[0].drawings[0].stroke_width, 2.);
        for field in ["coordinateSpace", "styleUnit"] {
            for replacement in [None, Some(Value::Null), Some(json!("encodedPixelsWrong"))] {
                let mut bad = good.clone();
                if let Some(value) = replacement {
                    bad["groups"][0][field] = value;
                } else {
                    bad["groups"][0].as_object_mut().unwrap().remove(field);
                }
                assert!(!validator.is_valid(&bad));
                assert!(parse_batch(&bad.to_string(), &scope.input).is_err());
            }
        }
        for replacement in [None, Some(Value::Null), Some(json!("center"))] {
            let mut bad = good.clone();
            if let Some(value) = replacement {
                bad["groups"][0]["objects"][0]["anchor"] = value;
            } else {
                bad["groups"][0]["objects"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("anchor");
            }
            assert!(!validator.is_valid(&bad));
            assert!(parse_batch(&bad.to_string(), &scope.input).is_err());
        }
        let max = schema.pointer("/properties/groups/items/oneOf/0/properties/objects/items/oneOf/2/properties/points/items/properties").unwrap();
        assert_eq!(max["x"]["maximum"], 1000);
        assert_eq!(max["y"]["maximum"], 1000);
        for (field, value) in [
            ("coordinateSpace", json!({"normalized1000":null})),
            ("styleUnit", json!({"encodedPixels":null})),
        ] {
            let mut bad = good.clone();
            bad["groups"][0][field] = value;
            assert!(!validator.is_valid(&bad));
            assert!(parse_batch(&bad.to_string(), &scope.input).is_err());
        }
        let mut bad = good.clone();
        bad["groups"][0]["objects"][0]["anchor"] = json!({"topLeft":null});
        assert!(!validator.is_valid(&bad));
        assert!(parse_batch(&bad.to_string(), &scope.input).is_err());
    }

    #[test]
    fn legacy_737_pixel_failure_and_manifest_hash_are_not_reinterpreted() {
        let mut manifest = input().manifest().clone();
        manifest.targets[0].encoded = mewu_core::VisualPixelSize {
            width: 1024,
            height: 683,
        };
        let input = VerifiedVisualInput::new(manifest).unwrap();
        let original = input.clone();
        let mut bad = args(&input);
        bad["groups"][0]["objects"][0]["points"][0] = json!({"x":856,"y":737});
        assert!(parse_batch(&bad.to_string(), &input)
            .unwrap_err()
            .contains("批注坐标超出附件内容"));
        for (field, value) in [
            ("coordinateSpace", json!("normalized1000")),
            ("styleUnit", json!("encodedPixels")),
            ("coordinateSpace", Value::Null),
        ] {
            let mut spoof = args(&input);
            spoof["groups"][0][field] = value;
            assert!(parse_batch(&spoof.to_string(), &input).is_err());
        }
        let mut spoof = args(&input);
        spoof["groups"][0]["objects"][0]["anchor"] = Value::Null;
        assert!(parse_batch(&spoof.to_string(), &input).is_err());
        assert_eq!(input, original);
        assert_eq!(input.sha256(), original.sha256());
    }

    #[test]
    fn mixed_targets_keep_their_own_units_dimensions_and_style_scale() {
        let mut manifest = input().manifest().clone();
        let mut second = manifest.targets[0].clone();
        second.handle = uuid::Uuid::new_v4().to_string();
        second.attachment_id = uuid::Uuid::new_v4().to_string();
        second.attachment_ordinal = 1;
        second.region_id = uuid::Uuid::new_v4().to_string();
        second.profile_id = VisualCoordinateProfile::ExplicitNormalized1000V2
            .id()
            .into();
        second.encoded = mewu_core::VisualPixelSize {
            width: 150,
            height: 100,
        };
        manifest.targets.push(second.clone());
        let scope = scope(VerifiedVisualInput::new(manifest).unwrap());
        let mut good = args(&scope.input);
        let mut normalized = normalized_args(&scope.input)["groups"][0].clone();
        normalized["target"] = json!(second.handle);
        normalized["objects"][0]["points"][0] = json!({"x":1000,"y":500});
        good["groups"].as_array_mut().unwrap().push(normalized);
        let definition = scope.definition();
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(definition.pointer("/function/parameters").unwrap())
            .unwrap();
        assert!(validator.is_valid(&good));
        let result = parse_batch(&good.to_string(), &scope.input).unwrap();
        assert_eq!(
            result.groups[0].drawings[0].points[0],
            DrawingPoint { x: 320., y: 340. }
        );
        assert_eq!(
            result.groups[1].drawings[0].points[0],
            DrawingPoint { x: 900., y: 500. }
        );
        assert_eq!(result.groups[0].drawings[0].font_size, Some(32.));
        assert_eq!(result.groups[1].drawings[0].font_size, Some(64.));
        let mut bad = good.clone();
        bad["groups"][1]["target"] = good["groups"][0]["target"].clone();
        assert!(!validator.is_valid(&bad));
        assert!(parse_batch(&bad.to_string(), &scope.input).is_err());
        let mut bad = good.clone();
        bad["groups"][0]["objects"][0]["points"][0]["y"] = json!(201);
        assert!(!validator.is_valid(&bad));
        assert!(parse_batch(&bad.to_string(), &scope.input).is_err());
    }

    #[test]
    fn normalized_odd_geometry_preserves_fractional_crop_tile_and_rejects_unknown_profile() {
        let mut manifest = normalized_input().manifest().clone();
        let target = &mut manifest.targets[0];
        target.source.source_size = mewu_core::VisualPixelSize {
            width: 3100,
            height: 4100,
        };
        target.crop = mewu_core::VisualPixelRect {
            x: 23,
            y: 41,
            width: 3001,
            height: 4003,
        };
        target.resized = mewu_core::VisualPixelSize {
            width: 1500,
            height: 2001,
        };
        target.tile = mewu_core::VisualPixelRect {
            x: 0,
            y: 1000,
            width: 1500,
            height: 1001,
        };
        target.encoded = mewu_core::VisualPixelSize {
            width: 1024,
            height: 683,
        };
        let input = VerifiedVisualInput::new(manifest.clone()).unwrap();
        let mut value = normalized_args(&input);
        value["groups"][0]["objects"][0]["points"][0] = json!({"x":333,"y":250});
        let point =
            &parse_batch(&value.to_string(), &input).unwrap().groups[0].drawings[0].points[0];
        assert!((point.x - (23. + 0.333 * 3001.)).abs() < 1e-9);
        assert!((point.y - (41. + (1000. + 0.25 * 1001.) * 4003. / 2001.)).abs() < 1e-9);
        for coordinate in [-0.1, 1000.1] {
            value["groups"][0]["objects"][0]["points"][0]["y"] = json!(coordinate);
            assert!(parse_batch(&value.to_string(), &input).is_err());
        }
        manifest.targets[0].profile_id = "future-unverified-profile".into();
        let unknown = VerifiedVisualInput::new(manifest).unwrap();
        assert!(parse_batch(&args(&unknown).to_string(), &unknown).is_err());
        assert!(RunScope::new(
            uuid::Uuid::new_v4().to_string(),
            VisualAnnotationGrant {
                plugin_id: "mewu.annotations".into(),
                plugin_revision: 1,
                contribution_id: "annotate".into()
            },
            unknown
        )
        .is_err());
    }
}
