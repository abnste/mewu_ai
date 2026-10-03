// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;

namespace mewu_ai_Assistant.Views;

public static class CaptureToolbarCaption
{
    public static readonly DependencyProperty TextProperty = DependencyProperty.RegisterAttached(
        "Text", typeof(string), typeof(CaptureToolbarCaption), new PropertyMetadata(string.Empty));

    public static string GetText(DependencyObject element) => (string)element.GetValue(TextProperty);
    public static void SetText(DependencyObject element, string value) => element.SetValue(TextProperty, value);
}

public partial class CaptureOverlayWindow
{
    private void InitializeCaptureToolbarCaptions()
    {
        void Caption(System.Windows.Controls.Button button, string chinese, string english)
            => CaptureToolbarCaption.SetText(button, L(chinese, english));

        Caption(ReferenceButton, "引用", "Ref");
        Caption(AddRegionButton, "添加", "Add");
        Caption(RemoveRegionButton, "删除", "Delete");
        Caption(DrawButton, "标注", "Draw");
        Caption(OcrButton, "文字", "OCR");
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
    }
}
