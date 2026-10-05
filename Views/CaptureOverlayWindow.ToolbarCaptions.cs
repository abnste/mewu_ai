// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;

namespace mewu_ai_Assistant.Views;

public static class CaptureToolbarCaption
{
    public static readonly DependencyProperty IsVisibleProperty = DependencyProperty.RegisterAttached(
        "IsVisible", typeof(bool), typeof(CaptureToolbarCaption), new FrameworkPropertyMetadata(true,FrameworkPropertyMetadataOptions.Inherits));
    public static bool GetIsVisible(DependencyObject element) => (bool)element.GetValue(IsVisibleProperty);
    public static void SetIsVisible(DependencyObject element, bool value) => element.SetValue(IsVisibleProperty, value);

    public static readonly DependencyProperty TextProperty = DependencyProperty.RegisterAttached(
        "Text", typeof(string), typeof(CaptureToolbarCaption), new PropertyMetadata(string.Empty));

    public static string GetText(DependencyObject element) => (string)element.GetValue(TextProperty);
    public static void SetText(DependencyObject element, string value) => element.SetValue(TextProperty, value);
}

public partial class CaptureOverlayWindow
{
    private void InitializeCaptureToolbarCaptions()
    {
        CaptureToolbarCaption.SetIsVisible(this,_host.Settings.ShowToolbarCaptions);
        void Caption(System.Windows.Controls.Button button, string chinese, string english)
        {
            CaptureToolbarCaption.SetText(button, L(chinese, english));
            button.SetResourceReference(StyleProperty,button==ReferenceButton||button==DrawingDoneButton?"CaptionReferenceButton":"CaptionToolbarButton");
        }

        Caption(ReferenceButton, "引用", "Ref");
        Caption(AddRegionButton, "添加", "Add");
        Caption(RemoveRegionButton, "删除", "Delete");
        Caption(DrawButton, "标注", "Draw");
        Caption(OcrButton, "文字", "OCR");
        OcrButton.ToolTip = L("原位文字识别 (O)；右键扫描填充", "Recognize text (O); right-click to scan and fill");
        Caption(TranslateButton, "翻译", "Translate");
        Caption(TableButton, "表格", "Table");
        Caption(LongCaptureButton, "长图", "Scroll");
        Caption(RecordButton, "录屏", "Record");
        Caption(VideoPlayButton, "播放", "Play");
        Caption(CopyButton, "复制", "Copy");
        Caption(SaveButton, "保存", "Save");
        Caption(PinButton, "置顶", "Pin");
        Caption(DingTalkButton, "钉钉", "DingTalk");
        Caption(FeishuButton, "飞书", "Feishu");
        Caption(ObsidianButton, "笔记", "Notes");
        Caption(ImaButton, "ima", "ima");
        Caption(DrawingSelectButton, "选择", "Select");
        Caption(DrawingPenButton, "画笔", "Pen");
        Caption(DrawingEraserButton, "橡皮", "Eraser");
        Caption(DrawingHighlightButton, "荧光", "Marker");
        Caption(DrawingLineButton, "直线", "Line");
        Caption(DrawingArrowButton, "箭头", "Arrow");
        Caption(DrawingRectangleButton, "矩形", "Box");
        Caption(DrawingEllipseButton, "椭圆", "Oval");
        Caption(DrawingTextButton, "文字", "Text");
        Caption(DrawingNumberButton, "序号", "Number");
        Caption(DrawingMosaicButton, "马赛克", "Mosaic");
        Caption(DrawingSeamlessEraseButton, "提取", "Lift");
        Caption(DrawingHealButton, "消除", "Heal");
        Caption(DrawingMarkButton, "重点", "Mark");
        Caption(DrawingUndoButton, "撤回", "Undo");
        Caption(DrawingRedoButton, "重做", "Redo");
        Caption(DrawingClearButton, "清空", "Clear");
        Caption(DrawingDoneButton, "确定", "Done");
        Caption(DrawingRedButton, "红色", "Red");
        Caption(DrawingBlueButton, "蓝色", "Blue");
        Caption(DrawingColorButton, "颜色", "Color");
    }
}
