// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private bool _syncingDrawingControls;

    private void UpdateDrawingToolbarPresentation()
    {
        // The second row contains real properties and actions. Tools without
        // properties collapse to the action group instead of reserving hint space.
        DrawingSelectionHint.Visibility = Visibility.Collapsed;
        DrawingToolHint.Visibility = Visibility.Collapsed;
        DrawingSelectionHint.Text = DrawingToolHint.Text = string.Empty;
        DrawingActionsSeparator.Visibility =
            DrawingColorControls.Visibility == Visibility.Visible ||
            DrawingStrokeControls.Visibility == Visibility.Visible ||
            DrawingNumberControls.Visibility == Visibility.Visible ||
            DrawingTextControls.Visibility == Visibility.Visible
                ? Visibility.Visible : Visibility.Collapsed;

        SetLabel(DrawingSeamlessEraseButton,
            "无痕提取：框选内容成为可移动或删除的透明图层，原处自动补齐背景",
            "Seamless lift: turn selected content into a transparent layer to move or delete; restore the background beneath it",
            "无痕提取工具", "Seamless lift tool");
        SetLabel(DrawingUndoButton, "撤回", "Undo", "撤回标注", "Undo annotation");
        SetLabel(DrawingHealButton, "涂抹消除：刷过要去掉的内容，松手自动修补背景；可调整笔刷大小",
            "Healing brush: paint over unwanted content, release to repair the background; adjustable brush size", "涂抹消除工具", "Healing brush tool");
        SetLabel(DrawingHighlightButton, "荧光笔：突出背景，保护原有文字", "Highlighter: tint the background while protecting text", "荧光笔工具", "Highlighter tool");
        SetLabel(DrawingMarkButton, "重点高亮：突出背景，保护原有文字；可移动、调整大小和改色",
            "Highlight region: tint the background while protecting text; move, resize or recolor", "重点高亮工具", "Highlight region tool");
        SetLabel(DrawingRedoButton, "重做", "Redo", "重做标注", "Redo annotation");
        SetLabel(DrawingClearButton, "删除全部标注", "Delete all annotations", "删除全部标注", "Delete all annotations");
        SetLabel(DrawingDoneButton, "确定", "Done", "完成标注", "Finish drawing");
        SetLabel(DrawingNumberButton, "序号：点击连续标记，可在下方调整下个序号", "Number: click to place consecutive labels; adjust the next number below", "序号标注工具", "Number label tool");

        static void SetLabel(Button button, string tooltipZh, string tooltipEn, string nameZh, string nameEn)
        {
            button.ToolTip = LocalizationService.T(tooltipZh, tooltipEn);
            AutomationProperties.SetName(button, LocalizationService.T(nameZh, nameEn));
        }
    }

    private sealed record StrokeStyleDrawingAction(Stroke Stroke, DrawingAttributes Before, DrawingAttributes After) : DrawingAction;
    private sealed record ElementStyleDrawingAction(DrawingElementSpec Before, DrawingElementSpec After) : DrawingAction;

    private DrawingElementSpec? SelectedDrawingElement(SelectionItem item) =>
        _selectedDrawingElementId is { } id ? item.DrawingElements.FirstOrDefault(element => element.Id == id) : null;

    private void ApplySelectedDrawingColor(SelectionItem item)
    {
        if (_drawTool is not (DrawTool.Select or DrawTool.Text)) return;
        if (ChangeSelectedRegionMarkColor(item)) return;
        if (_selectedDrawingStroke is { } stroke && item.Markup.Strokes.Contains(stroke))
        {
            if (stroke.DrawingAttributes.Color == _drawColor) return;
            var before = stroke.DrawingAttributes.Clone();
            stroke.DrawingAttributes.Color = _drawColor;
            item.DrawingOrder.Add(new StrokeStyleDrawingAction(stroke, before, stroke.DrawingAttributes.Clone()));
            item.DrawingRedo.Clear();
            MarkDrawingChanged(item);
            return;
        }
        var selected = SelectedDrawingElement(item);
        var updated = selected switch
        {
            TextDrawingElement text => (DrawingElementSpec)(text with { Color = _drawColor }),
            NumberDrawingElement number => number with { Color = _drawColor },
            _ => selected
        };
        if (selected is not null && updated is not null) CommitDrawingElementProperty(item, selected, updated);
    }

    private void CommitDrawingElementProperty(SelectionItem item, DrawingElementSpec before, DrawingElementSpec after)
    {
        if (Equals(before, after) || !ReplaceDrawingElement(item, after)) return;
        UpdateDrawingElementVisual(item, after);
        item.DrawingOrder.Add(new ElementStyleDrawingAction(before, after));
        item.DrawingRedo.Clear();
        MarkDrawingChanged(item);
        ShowDrawingObjectSelection(item);
    }

    private static bool ApplyDrawingElementStyle(SelectionItem item, DrawingElementSpec style)
    {
        var current = item.DrawingElements.FirstOrDefault(element => element.Id == style.Id);
        var updated = (current, style) switch
        {
            (TextDrawingElement text, TextDrawingElement values) => (DrawingElementSpec)(text with
            {
                FontFamily = values.FontFamily, FontSize = values.FontSize,
                Color = values.Color, Highlight = values.Highlight
            }),
            (NumberDrawingElement number, NumberDrawingElement values) => number with { Color = values.Color },
            _ => null
        };
        return updated is not null && ReplaceDrawingElement(item, updated);
    }

    private void SyncDrawingPropertyControls(SelectionItem item)
    {
        if (_syncingDrawingControls) return;
        _syncingDrawingControls = true;
        try
        {
            var selected = SelectedDrawingElement(item);
            if (selected is TextDrawingElement text)
            {
                _drawColor = text.Color;
                _drawFontFamily = text.FontFamily;
                _drawFontSize = text.FontSize;
                _drawTextHighlight = text.Highlight;
            }
            else if (selected is NumberDrawingElement number) _drawColor = number.Color;
            else if (SelectedDrawingRegionMark(item) is { } mark) _drawColor = mark.Color;
            else if (_selectedDrawingStroke is { } stroke && item.Markup.Strokes.Contains(stroke)) _drawColor = stroke.DrawingAttributes.Color;
            UpdateDrawingContextControls(item);
            ApplyCurrentDrawingAttributes(item);
            if (_drawingMode && DrawingToolbar.Visibility == Visibility.Visible) PositionFloatingBar(DrawingToolbar, item);
        }
        finally { _syncingDrawingControls = false; }
    }

    private void FocusDrawingText(SelectionItem item, TextBox editor)
    {
        if (!_drawingMode || _drawTool != DrawTool.Text || !ReferenceEquals(item, Active) ||
            editor.Tag is not Guid id || !item.DrawingElements.Any(element => element.Id == id)) return;
        _selectedDrawingElementId = id;
        _selectedDrawingStroke = null;
        _selectedDrawingRegionMark = null;
        UpdateDrawingTextInput(item);
        editor.BorderBrush = new SolidColorBrush(Color.FromRgb(108, 124, 238));
        SyncDrawingPropertyControls(item);
    }

    private void DrawingTextLostFocus(SelectionItem item, TextBox editor)
    {
        editor.BorderBrush = Brushes.Transparent;
        _ = Dispatcher.BeginInvoke(DispatcherPriority.Input, new Action(() =>
        {
            if (_closed || !_selections.Contains(item) || editor.IsKeyboardFocusWithin || _drawingModalOpen ||
                DrawingFontFamily.IsDropDownOpen || DrawingFontSize.IsDropDownOpen || DrawingStrokeWidth.IsDropDownOpen) return;
            // A toolbar interaction keeps the editing target, including a new text draft.
            if (Keyboard.FocusedElement is DependencyObject focus && IsInside(focus, DrawingToolbar)) return;
            if (editor.Tag is Guid id && ReferenceEquals(FindDrawingElementVisual(item, id), editor) &&
                item.DrawingElements.Any(element => element.Id == id &&
                element is TextDrawingElement text && string.IsNullOrWhiteSpace(text.Text))) RemoveEmptyDrawingText(item);
        }));
    }

    private static double DrawingNumberFontSize(NumberDrawingElement number)
    {
        var digits = number.Number.ToString(System.Globalization.CultureInfo.InvariantCulture).Length;
        return Math.Clamp(number.Diameter * Math.Min(.68, 1.08 / digits), 6, 80);
    }

    private static bool DrawingActionTargets(DrawingAction action, HashSet<Guid> ids) => action switch
    {
        LiftDrawingAction lifted => ids.Contains(lifted.Background.Id) || ids.Contains(lifted.Content.Id),
        ElementDrawingAction created => ids.Contains(created.Element.Id),
        ElementRemovalDrawingAction removed => ids.Contains(removed.Element.Id),
        ElementMoveDrawingAction changed => ids.Contains(changed.Before.Id),
        ElementStyleDrawingAction styled => ids.Contains(styled.Before.Id),
        _ => false
    };
}
