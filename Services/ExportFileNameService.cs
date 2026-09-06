// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Services;

internal static class ExportFileNameService
{
    internal static string Screenshot(DateTime timestamp)=>$"截图_{timestamp:yyyyMMdd_HHmmss}";
    internal static string Recording(DateTime timestamp)=>$"录屏_{timestamp:yyyyMMdd_HHmmss}";
}
