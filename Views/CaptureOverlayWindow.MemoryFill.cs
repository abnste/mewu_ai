// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Interop;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.OCR;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private async void ScanFill(object? sender, RoutedEventArgs e)
    {
        if (RejectIfOverlayOperationBusy()) return;
        new PrivacyLogger().Info("MemoryScanFill", "button-clicked");
        if (Active is not { IsImplicit: false } item || item.VideoPath is not null)
        {
            PromptStatus.Text = L("请先圈选登录表单区域。", "Select a form region first.");
            return;
        }
        var memories = _host.Settings.MemoryEntries ?? [];
        if (!memories.Any(entry => entry is { Enabled: true, Keywords.Count: > 0 }))
        {
            PromptStatus.Text = L("请先在设置 → 记忆中添加关键词和值。", "Add keywords and values under Settings → Memory first.");
            return;
        }
        var operation = BeginOverlayOperation(L("正在扫描表单并匹配记忆…", "Scanning the form and matching memories…"));
        try
        {
            new PrivacyLogger().Info("MemoryScanFill", $"started; memories={memories.Count}; region={item.Bounds.Width:F0}x{item.Bounds.Height:F0}");
            var image = RenderSelectionImage(item, false, false, false);
            var document = await new WindowsOcrService().RecognizeAsync(image, operation.Token).ConfigureAwait(true);
            if (!IsOverlayOperationActive(operation, item)) return;
            var matches = MemoryStore.Match(memories, document.Text);
            new PrivacyLogger().Info("MemoryScanFill", $"ocrChars={document.Text.Length}; lines={document.Lines.Count}; matches={matches.Count}");
            if (matches.Count == 0)
            {
                PromptStatus.Text = L("未识别到已保存的关键词。", "No saved keywords were found.");
                return;
            }
            var screen = ScreenCoordinateService.ToScreenRect(ToPixelRect(item.Bounds), _frame.OriginX, _frame.OriginY);
            var target = item.SnapshotTarget ?? ResolveMemoryTarget(screen);
            if (target is null || !target.IsCurrent() || target.Handle == new WindowInteropHelper(this).Handle.ToInt64())
            {
                PromptStatus.Text = L("未能获取当前窗口的输入控件，请重新圈选。", "Could not access the input controls in this window. Select again.");
                return;
            }
            var inputs = await Task.Run(() => MemoryFillService.FindInputs(target, screen, operation.Token), operation.Token).ConfigureAwait(true);
            if (!IsOverlayOperationActive(operation, item) || !target.IsCurrent()) return;
            new PrivacyLogger().Info("MemoryScanFill", $"inputs={inputs.Count}; target={target.Handle}");
            if (inputs.Count == 0)
            {
                PromptStatus.Text = L("未找到可填充的输入框；请点击输入框后重试。", "No fillable input was found. Click an input and retry.");
                return;
            }
            var filled = 0;
            var usedInputs = new HashSet<MemoryInputCandidate>();
            foreach (var match in matches)
            {
                if (!IsOverlayOperationActive(operation, item) || !target.IsCurrent()) return;
                var value = MemoryStore.Read(match.Entry);
                if (string.IsNullOrEmpty(value)) continue;
                var keyword = MemoryStore.Normalize(match.Keyword);
                var line = document.Lines.FirstOrDefault(candidate => MemoryStore.Normalize(candidate.Text).Contains(keyword, StringComparison.Ordinal));
                var anchor = line is null
                    ? new Point(screen.X + screen.Width / 2d, screen.Y + screen.Height / 2d)
                    : new Point(
                        screen.X + (line.X + line.Width / 2d) * screen.Width / Math.Max(1, image.PixelWidth),
                        screen.Y + (line.Y + line.Height / 2d) * screen.Height / Math.Max(1, image.PixelHeight));
                var available = inputs.Where(input => !input.IsReadOnly && !usedInputs.Contains(input)).ToArray();
                var requiresPassword = match.Entry.Sensitive || string.Equals(match.Entry.FieldKind, "password", StringComparison.OrdinalIgnoreCase);
                var pool = available.Where(input => input.IsPassword == requiresPassword).ToArray();
                var candidate = pool.OrderByDescending(input => MemoryFillService.Score(match.Entry, input, anchor, screen)).FirstOrDefault();
                if (candidate is null) continue;
                var confidence = MemoryFillService.Score(match.Entry, candidate, anchor, screen);
                var threshold = Math.Clamp(_host.Settings.MemoryConfidenceThreshold, .5, .98);
                var strict = string.Equals(_host.Settings.MemoryDetectionPlan, "strict", StringComparison.OrdinalIgnoreCase);
                if ((strict || confidence < threshold) && !ConfirmMemoryCandidate(match.Entry, candidate, confidence)) continue;
                if (!IsOverlayOperationActive(operation, item) || !target.IsCurrent()) return;
                var fillOk = MemoryFillService.TryFill(candidate, value, new IntPtr(target.Handle));
                new PrivacyLogger().Info("MemoryScanFill", $"fill kind={match.Entry.FieldKind}; bounds={candidate.Bounds.Left:F0},{candidate.Bounds.Top:F0},{candidate.Bounds.Width:F0}x{candidate.Bounds.Height:F0}; confidence={confidence:F2}; success={fillOk}");
                if (fillOk) { usedInputs.Add(candidate); filled++; }
            }
            PromptStatus.Text = filled == 0 ? L("未能安全确定输入框，未执行填充。", "No input could be matched safely; nothing was filled.") : string.Format(System.Globalization.CultureInfo.CurrentCulture, L("已自动填充 {0} 项。", "Automatically filled {0} field(s)."), filled);
        }
        catch (OperationCanceledException) when (operation.IsCancellationRequested) { }
        catch (Exception ex)
        {
            new PrivacyLogger().Info("MemoryScanFill", ex.GetType().Name);
            PromptStatus.Text = L("扫描填充失败，请重新圈选后重试。", "Scan-and-fill failed. Select the region again and retry.");
        }
        finally { EndOverlayOperation(operation); }
    }

    private ApplicationSnapshotTarget? ResolveMemoryTarget(ScreenRect screen)
    {
        if (screen.IsEmpty) return null;
        var handle = new WindowInteropHelper(this).Handle;
        var hit = _windowSnap.FindFastTargetAt(screen.X + screen.Width / 2, screen.Y + screen.Height / 2, handle);
        return hit is null ? null : ApplicationSnapshotTarget.FromWindow(hit.Handle);
    }

    private bool ConfirmMemoryCandidate(MemoryEntry entry, MemoryInputCandidate candidate, double confidence)
    {
        var field = entry.Sensitive ? L("敏感字段", "sensitive field") : L("字段", "field");
        return MewuDialogWindow.ShowChoice(this, L("确认扫描填充", "Confirm scan fill"),
            string.Format(System.Globalization.CultureInfo.CurrentCulture, L("检测到 {0}，匹配置信度 {1:P0}。是否填充？", "Detected a {0} with {1:P0} confidence. Fill it?"), field, confidence),
            L("填充", "Fill"), string.Empty, L("取消", "Cancel")) == MewuDialogResult.Primary;
    }
}
