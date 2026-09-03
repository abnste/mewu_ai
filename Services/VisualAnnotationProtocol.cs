namespace mewu_ai_Assistant.Services;

internal static class VisualAnnotationProtocol
{
    internal const string Version="mewu.visual-annotations/1";
    internal const int MaximumAnnotations=48;
    internal const int MaximumPointsPerPath=128;
    internal const int MaximumKeyframesPerAnnotation=128;
    internal const int MaximumCallouts=12;
    internal const int MaximumAnnotationTextLength=500;

    internal const string SystemInstruction="""
分析屏幕附件时只能返回一个 JSON 根对象，禁止 Markdown 围栏和 JSON 外说明。根对象必须是 {annotationProtocol:"mewu.visual-annotations/1",answer:string,annotationMode:"preserve"|"append"|"replace",annotations:array}。
图片和视频共用视觉标注协议。每条标注包含 target:{regionIndex,referenceHandle}、kind、geometry 或 timeline、可选 style、可选 content 与 label。kind 只能是 callout、pen、highlighter、rectangle、ellipse、arrow、text、number、mosaic。
图片使用 geometry。矩形类使用 geometry:{coordinateSpace:"normalized",rect:{x,y,width,height}}；pen、highlighter、arrow 使用 geometry:{coordinateSpace:"normalized",points:[{x,y},...]}。坐标都是附件自身 0 到 1 的归一化值。
视频必须使用 timeline:{startTime,endTime,keyframes:[{time,geometry},...]}，时间为秒；单点令起止时间相等且一个关键帧，动作区间至少两个严格递增关键帧，只在目标边界或运动方向实际变化时增加关键帧，单条最多 128 个。禁止给视频返回无时间轴的静态标注。
style 允许 color:#RRGGBB、strokeWidth:0.001..0.1、opacity:0.05..1、filled:boolean、fontSize:0.01..0.2。text 使用 content:{text}，number 使用 content:{number}。callout 的 geometry.rect 永远是被标目标的紧贴边界，label 是独立气泡文字；目标含可见文字时，label 必须原样包含一段最短且唯一的目标文字，再补充简短说明，便于客户端用本地 OCR 校准。客户端会画一个红色目标框并自动避让气泡。不要为同一目标再返回重叠的 rectangle/ellipse，也不要在 answer 内描述像素坐标、归一化数值或“请画框”等绘制步骤；需要可见标记时必须写入 annotations。
所有 target.referenceHandle 必须原样复制附件清单中的不可变句柄；regionIndex 是完整图片与视频上传顺序。坏掉、越界、未知句柄或类型不匹配的单条标注会被丢弃。最多返回 48 条标注，其中 callout 最多 12 条，每条路径最多 128 点。只在确有帮助时使用马赛克，不能遮挡答题所需信息。
附件清单标记 hasExistingAiAnnotations=true 时，当前图片像素已经包含上一轮 AI 标注。若本轮只回答问题且无需改变标注，annotationMode 必须为 preserve 且 annotations=[]；若要在旧标注上增加内容，使用 append 且 annotations 只返回新增项；只有决定重新标注时才使用 replace，并返回替换后的完整新标注。禁止在普通追问时无故清空或重复描摹已有标注。
""";
}
