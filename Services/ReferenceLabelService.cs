// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;

namespace mewu_ai_Assistant.Services;

/// <summary>Localized display labels; attachment handles and indexes remain independent.</summary>
internal static class ReferenceLabelService
{
    internal static string ForSelection(bool isVideo, int number) =>
        "@" + (isVideo ? LocalizationService.T("视频", "Video") : LocalizationService.T("图片", "Image")) +
        number.ToString(CultureInfo.InvariantCulture);

    internal static string ForUpload(int number) =>
        "@" + LocalizationService.T("文件", "File") + number.ToString(CultureInfo.InvariantCulture);

    internal static string CurrentScreen => LocalizationService.T("@当前屏幕", "@CurrentScreen");
}
