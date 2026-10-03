// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Input;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private const int MaximumDrawingNumber = 9999;

    private static int FollowingDrawingNumber(int number) => Math.Min(MaximumDrawingNumber, number + 1);

    private void UpdateDrawingNumberControls(SelectionItem item)
    {
        if (DrawingNumberValue is null || !ReferenceEquals(item, Active)) return;
        var wasSyncing = _syncingDrawingControls;
        _syncingDrawingControls = true;
        try
        {
            DrawingNumberLabel.Text = LocalizationService.T("下个序号", "Next number");
            DrawingNumberValue.ToolTip = LocalizationService.T("下一枚序号，可输入 1–9999", "Next number to place, from 1 to 9999");
            AutomationProperties.SetName(DrawingNumberValue, LocalizationService.T("下个序号", "Next number"));
            DrawingNumberDecrease.ToolTip = LocalizationService.T("减小序号", "Decrease next number");
            DrawingNumberIncrease.ToolTip = LocalizationService.T("增大序号", "Increase next number");
            AutomationProperties.SetName(DrawingNumberDecrease, (string)DrawingNumberDecrease.ToolTip);
            AutomationProperties.SetName(DrawingNumberIncrease, (string)DrawingNumberIncrease.ToolTip);
            DrawingNumberValue.Text = Math.Clamp(item.NextDrawingNumber, 1, MaximumDrawingNumber).ToString(CultureInfo.InvariantCulture);
            DrawingNumberDecrease.IsEnabled = item.NextDrawingNumber > 1;
            DrawingNumberIncrease.IsEnabled = item.NextDrawingNumber < MaximumDrawingNumber;
        }
        finally { _syncingDrawingControls = wasSyncing; }
    }

    private void DrawingNumberValueChanged(object sender, TextChangedEventArgs e)
    {
        if (_syncingDrawingControls || DrawingNumberValue is null || _drawTool != DrawTool.Number || Active is not { } item) return;
        if (!int.TryParse(DrawingNumberValue.Text, NumberStyles.None, CultureInfo.InvariantCulture, out var value) || value is < 1 or > MaximumDrawingNumber) return;
        // This is a tool preference, not a new annotation or a rewrite of existing labels.
        item.NextDrawingNumber = value;
        item.DrawingNumberPreference = Guid.NewGuid();
        DrawingNumberDecrease.IsEnabled = value > 1;
        DrawingNumberIncrease.IsEnabled = value < MaximumDrawingNumber;
    }

    private void DrawingNumberValueLostFocus(object sender, KeyboardFocusChangedEventArgs e)
    {
        if (Active is { } item) UpdateDrawingNumberControls(item);
    }

    private void DrawingNumberDecreaseClick(object sender, RoutedEventArgs e) => StepDrawingNumber(-1);
    private void DrawingNumberIncreaseClick(object sender, RoutedEventArgs e) => StepDrawingNumber(1);

    private void StepDrawingNumber(int delta)
    {
        if (Active is not { } item) return;
        var next = Math.Clamp(item.NextDrawingNumber + delta, 1, MaximumDrawingNumber);
        if (next != item.NextDrawingNumber) item.DrawingNumberPreference = Guid.NewGuid();
        item.NextDrawingNumber = next;
        UpdateDrawingNumberControls(item);
    }

    private void DrawingNumberKeyDown(KeyEventArgs e)
    {
        if (e.Key is Key.Up or Key.Down)
        {
            StepDrawingNumber(e.Key == Key.Up ? 1 : -1);
            DrawingNumberValue.SelectAll();
            e.Handled = true;
        }
        else if (e.Key is Key.Enter or Key.Escape)
        {
            if (Active is { } item) UpdateDrawingNumberControls(item);
            Keyboard.Focus(Root);
            e.Handled = true;
        }
        // Text editing, including Delete and Ctrl+Z, keeps native TextBox handling.
    }
}
