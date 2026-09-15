// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Shell;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public sealed partial class SettingsWindow
{
    private UIElement CreateWindowActions()
    {
        var actions = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Center
        };
        var maximize = CreateWindowActionButton("SettingsMaximizeButton");
        maximize.Margin = new Thickness(0, 0, 4, 0);
        void RefreshMaximizeAction()
        {
            var isMaximized = WindowState == WindowState.Maximized;
            var label = isMaximized
                ? LocalizationService.T("还原窗口", "Restore window")
                : LocalizationService.T("最大化窗口", "Maximize window");
            maximize.ToolTip = label;
            AutomationProperties.SetName(maximize, label);
            maximize.Content = new System.Windows.Shapes.Path
            {
                Data = Geometry.Parse(isMaximized
                    ? "M5,3 V1 H13 V9 H11 M1,5 H9 V13 H1 Z"
                    : "M1,1 H13 V13 H1 Z"),
                Width = 14, Height = 14, Stretch = Stretch.Uniform,
                Stroke = Foreground, StrokeThickness = 1.3,
                StrokeLineJoin = PenLineJoin.Round
            };
        }
        maximize.Click += (_, _) =>
        {
            if (WindowState == WindowState.Maximized) SystemCommands.RestoreWindow(this);
            else SystemCommands.MaximizeWindow(this);
        };
        StateChanged += (_, _) => RefreshMaximizeAction();
        RefreshMaximizeAction();
        actions.Children.Add(maximize);

        var minimize = CreateWindowActionButton("SettingsMinimizeButton");
        minimize.Content = new System.Windows.Shapes.Path
        {
            Data = Geometry.Parse("M2,7 H12"), Width = 14, Height = 14,
            Stretch = Stretch.Uniform, Stroke = Foreground, StrokeThickness = 1.4,
            StrokeStartLineCap = PenLineCap.Square, StrokeEndLineCap = PenLineCap.Square
        };
        minimize.ToolTip = LocalizationService.T("隐藏设置窗口", "Minimize settings window");
        AutomationProperties.SetName(minimize, minimize.ToolTip.ToString());
        minimize.Click += (_, _) => SystemCommands.MinimizeWindow(this);
        actions.Children.Insert(actions.Children.Count - 1, minimize);

        var close = CreateWindowActionButton("SettingsCloseButton");
        close.Content = CloseIcon();
        close.ToolTip = LocalizationService.T("关闭设置", "Close settings");
        AutomationProperties.SetName(close, LocalizationService.T("关闭设置窗口", "Close settings window"));
        close.Click += (_, _) => Close();
        actions.Children.Add(close);
        return actions;
    }

    private static Button CreateWindowActionButton(string automationId)
    {
        var button = new Button
        {
            Width = 34, Height = 34, MinWidth = 34, MinHeight = 34,
            Padding = new Thickness(0),
            HorizontalContentAlignment = HorizontalAlignment.Center,
            VerticalContentAlignment = VerticalAlignment.Center
        };
        button.SetResourceReference(StyleProperty, "RoundIconButton");
        AutomationProperties.SetAutomationId(button, automationId);
        // The native caption retains drag, double-click and system-menu
        // behavior while these controls remain normal, focusable WPF buttons.
        WindowChrome.SetIsHitTestVisibleInChrome(button, true);
        return button;
    }
}
