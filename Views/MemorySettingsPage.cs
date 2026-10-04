// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

internal sealed class MemorySettingsPage : ScrollViewer
{
    private readonly StackPanel _rows = new();
    private readonly List<Row> _items = [];
    private readonly AppSettings _settings;
    private readonly List<MemoryEntry> _originalEntries;
    private readonly ComboBox _plan = new();
    private readonly Slider _threshold = new() { Minimum = .5, Maximum = .98, TickFrequency = .01, IsSnapToTickEnabled = true, Width = 180 };
    private readonly TextBlock _thresholdLabel = new() { Width = 48, VerticalAlignment = VerticalAlignment.Center };
    private readonly CheckBox _visualModel = new();
    private readonly TextBox _modelPath = new();

    private sealed class Row
    {
        internal MemoryEntry Entry { get; }
        internal TextBox Keywords { get; } = new();
        internal TextBox Value { get; } = new() { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, MinHeight = 30 };
        internal ComboBox FieldKind { get; } = new();
        internal CheckBox Sensitive { get; } = new();
        internal CheckBox Enabled { get; } = new();
        internal Row(MemoryEntry entry) => Entry = entry;
    }

    internal MemorySettingsPage(AppSettings settings)
    {
        _settings = settings;
        _originalEntries = (settings.MemoryEntries ?? []).Select(Clone).ToList();
        VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled;
        Padding = new Thickness(20, 12, 20, 18);
        var root = new StackPanel();
        root.Children.Add(new TextBlock { Text = T("记忆", "Memory"), FontSize = 17, FontWeight = FontWeights.SemiBold });
        root.Children.Add(new TextBlock
        {
            Text = T("保存本机关键词和值，用于圈选区域后的扫描填充。所有值均使用 Windows DPAPI 加密，不会发送给 AI 或 MCP。", "Save local keyword/value mappings for scan-and-fill. Values are encrypted with Windows DPAPI and are never sent to AI or MCP."),
            TextWrapping = TextWrapping.Wrap, Foreground = new SolidColorBrush(Color.FromRgb(99, 112, 137)), Margin = new Thickness(0, 6, 0, 12)
        });
        var add = new Button { Content = T("添加记忆", "Add memory"), Padding = new Thickness(12, 6, 12, 6), HorizontalAlignment = HorizontalAlignment.Left };
        add.Click += (_, _) => AddRow(new MemoryEntry());
        root.Children.Add(add);
        var plan = new StackPanel { Margin = new Thickness(0, 14, 0, 4) };
        plan.Children.Add(new TextBlock { Text = T("扫描计划", "Detection plan"), FontWeight = FontWeights.SemiBold });
        _plan.Items.Add(new ComboBoxItem { Content = T("混合：规则 + UIA + 视觉模型（推荐）", "Hybrid: rules + UIA + visual model (recommended)"), Tag = "hybrid" });
        _plan.Items.Add(new ComboBoxItem { Content = T("规则优先：OCR + UIA + 几何", "Rules first: OCR + UIA + geometry"), Tag = "rules" });
        _plan.Items.Add(new ComboBoxItem { Content = T("严格：低置信度不自动填充", "Strict: never autofill low-confidence matches"), Tag = "strict" });
        _plan.SelectedValuePath = "Tag"; _plan.SelectedValue = settings.MemoryDetectionPlan;
        plan.Children.Add(_plan);
        var thresholdLine = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 6, 0, 0) };
        thresholdLine.Children.Add(new TextBlock { Text = T("自动填充阈值", "Autofill threshold"), Width = 110, VerticalAlignment = VerticalAlignment.Center });
        _threshold.Value = Math.Clamp(settings.MemoryConfidenceThreshold, .5, .98); thresholdLine.Children.Add(_threshold);
        _thresholdLabel.Text = $" {_threshold.Value:P0}"; thresholdLine.Children.Add(_thresholdLabel);
        _threshold.ValueChanged += (_, _) => _thresholdLabel.Text = $" {_threshold.Value:P0}";
        plan.Children.Add(thresholdLine);
        _visualModel.Content = T("启用本地视觉输入框模型（模型不可用时自动回退）", "Enable local visual input model (falls back automatically if unavailable)");
        _visualModel.IsChecked = settings.MemoryVisualModelEnabled; plan.Children.Add(_visualModel);
        _modelPath.Text = settings.MemoryVisualModelPath; _modelPath.ToolTip = T("可选 ONNX UI element detector 路径；不填写也可使用规则方案。", "Optional ONNX UI element detector path; rules work without it.");
        plan.Children.Add(AiSettingsForm.Field(T("视觉模型路径（可选）", "Visual model path (optional)"), _modelPath));
        root.Children.Add(plan);
        root.Children.Add(_rows);
        Content = root;
        PreviewMouseWheel += (_, e) =>
        {
            if (e.Handled) return;
            ScrollToVerticalOffset(VerticalOffset - Math.Sign(e.Delta) * 48);
            e.Handled = true;
        };
        foreach (var entry in settings.MemoryEntries ?? []) AddRow(Clone(entry));
    }

    internal void Apply(AppSettings settings)
    {
        var entries = new List<MemoryEntry>();
        foreach (var row in _items)
        {
            var keywords = row.Keywords.Text.Split([',', '，', ';', '；', '\n', '\r'], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).Distinct(StringComparer.OrdinalIgnoreCase).ToList();
            if (keywords.Count == 0) continue;
            row.Entry.Keywords = keywords;
            row.Entry.Sensitive = row.Sensitive.IsChecked == true;
            row.Entry.FieldKind = (row.FieldKind.SelectedValue as string) ?? "auto";
            if (row.Entry.FieldKind == "password") row.Entry.Sensitive = true;
            row.Entry.Enabled = row.Enabled.IsChecked != false;
            MemoryStore.EnsureCredentialId(row.Entry);
            if (row.Value.Text.Length > 0) MemoryStore.Save(row.Entry, row.Value.Text);
            else if (MemoryStore.Read(row.Entry) is null) throw new InvalidOperationException($"记忆项“{keywords[0]}”的值不能为空。");
            entries.Add(row.Entry);
        }
        var retained = entries.Select(entry => entry.CredentialId).ToHashSet(StringComparer.OrdinalIgnoreCase);
        foreach (var old in _originalEntries)
            if (!retained.Contains(old.CredentialId)) MemoryStore.Delete(old);
        settings.MemoryEntries = entries;
        settings.MemoryDetectionPlan = (_plan.SelectedValue as string) ?? "hybrid";
        settings.MemoryConfidenceThreshold = _threshold.Value;
        settings.MemoryVisualModelEnabled = _visualModel.IsChecked == true;
        settings.MemoryVisualModelPath = _modelPath.Text.Trim();
    }

    private void AddRow(MemoryEntry entry)
    {
        var row = new Row(entry);
        row.Keywords.Text = string.Join(", ", entry.Keywords ?? []);
        row.Sensitive.Content = T("敏感值（密码等）", "Sensitive value (password, etc.)");
        row.Sensitive.IsChecked = entry.Sensitive;
        row.Enabled.Content = T("启用", "Enabled");
        row.Enabled.IsChecked = entry.Enabled;
        row.Value.ToolTip = T("值将明文保存在本机设置文件中。", "The value is stored in plain text in the local settings file.");
        var existingValue = MemoryStore.Read(entry);
        if (!string.IsNullOrEmpty(existingValue)) row.Value.Text = existingValue;
        row.FieldKind.Items.Add(new ComboBoxItem { Content = T("自动", "Auto"), Tag = "auto" });
        row.FieldKind.Items.Add(new ComboBoxItem { Content = T("用户名", "Username"), Tag = "username" });
        row.FieldKind.Items.Add(new ComboBoxItem { Content = T("邮箱", "Email"), Tag = "email" });
        row.FieldKind.Items.Add(new ComboBoxItem { Content = T("密码", "Password"), Tag = "password" });
        row.FieldKind.Items.Add(new ComboBoxItem { Content = T("普通文本", "Text"), Tag = "text" });
        row.FieldKind.SelectedValuePath = "Tag";
        row.FieldKind.SelectedValue = entry.FieldKind is "username" or "email" or "password" or "text" ? entry.FieldKind : "auto";
        row.FieldKind.Width = 96; row.FieldKind.Margin = new Thickness(4, 0, 0, 0);
        var delete = new Button { Content = T("删除", "Delete"), Padding = new Thickness(8, 4, 8, 4), VerticalAlignment = VerticalAlignment.Top };
        delete.Click += (_, _) => { _items.Remove(row); _rows.Children.Remove((FrameworkElement)delete.Tag!); };
        var panel = new StackPanel { Margin = new Thickness(0, 8, 0, 4) };
        panel.Children.Add(new TextBlock { Text = T("关键词（逗号分隔）", "Keywords (comma separated)"), FontSize = 12 });
        var line = new DockPanel();
        DockPanel.SetDock(delete, Dock.Right); line.Children.Add(delete); line.Children.Add(row.Keywords); panel.Children.Add(line);
        var valueLine = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 4, 0, 0) };
        valueLine.Children.Add(new TextBlock { Text = T("值", "Value"), Width = 42, VerticalAlignment = VerticalAlignment.Center });
        row.Value.Width = 260; row.Value.Padding = new Thickness(7, 4, 7, 4); valueLine.Children.Add(row.Value); valueLine.Children.Add(row.FieldKind); valueLine.Children.Add(row.Sensitive); valueLine.Children.Add(row.Enabled); panel.Children.Add(valueLine);
        delete.Tag = panel; _items.Add(row); _rows.Children.Add(panel);
    }

    private static MemoryEntry Clone(MemoryEntry entry) => new() { Id = entry.Id, Keywords = [.. entry.Keywords ?? []], Sensitive = entry.Sensitive, Enabled = entry.Enabled, FieldKind = entry.FieldKind, CredentialId = entry.CredentialId, Value = entry.Value };
    private static string T(string zh, string en) => LocalizationService.T(zh, en);
}
