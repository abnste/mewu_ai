// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Services;

internal static class AccessibilityEvidencePolicy
{
    // A minimized conversation keeps its original pixels even after restoring.
    // Current desktop accessibility geometry is no longer evidence for that
    // image. Keep the image-based refinement path instead of recapturing it.
    internal static bool CanRefineFromLiveDesktop(bool hasCapturedImageOverride, bool conversationSessionFrozen)
        => !hasCapturedImageOverride && !conversationSessionFrozen;
}
