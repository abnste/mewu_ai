using System.Windows;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class CaptureOverlayPolicy
{
    internal const double PromptPreferredWidth = 574;
    internal const double PromptSideMargin = 16;
    internal const double PromptTopMargin = 16;
    internal const double PromptBottomMargin = 24;
    internal const double PromptReservedComposerHeight = 132;
    internal const int TranslationBatchLineLimit = 24;
    internal const int TranslationBatchCharacterLimit = 3200;
    internal static readonly TimeSpan RecordingStopTimeout = TimeSpan.FromSeconds(15);

    internal static IReadOnlyList<T> SelectSendTargets<T>(
        IEnumerable<T> selections,
        Func<T, bool> isImplicit,
        Func<T, bool> isReferenced)
    {
        var all = selections.ToList();
        var explicitSelections = all.Where(item => !isImplicit(item)).ToList();
        var eligible = explicitSelections.Count > 0 ? explicitSelections : all;
        var referenced = eligible.Where(isReferenced).ToList();
        return referenced.Count > 0 ? referenced : eligible;
    }

    internal static IReadOnlyList<(int RegionIndex,T Item)> SelectSpatialAnnotationTargets<T>(
        IReadOnlyList<T> attachments,
        Func<T,bool> isVideo) =>
        attachments
            .Select((item,index)=>(RegionIndex:index,Item:item))
            .Where(entry=>!isVideo(entry.Item))
            .ToList();

    internal static IReadOnlyList<TranslationBatch> CreateTranslationBatches(
        IReadOnlyList<string> lines,
        int lineLimit = TranslationBatchLineLimit,
        int characterLimit = TranslationBatchCharacterLimit)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(lineLimit, 1);
        ArgumentOutOfRangeException.ThrowIfLessThan(characterLimit, 1);

        var batches = new List<TranslationBatch>();
        var current = new List<string>();
        var currentStart = 0;
        var currentCharacters = 0;

        void Flush()
        {
            if (current.Count == 0)
                return;

            var outputBudget = Math.Clamp(
                512 + (int)Math.Ceiling(currentCharacters * 1.35),
                1024,
                4096);
            batches.Add(new TranslationBatch(currentStart, current.ToArray(), outputBudget));
            current.Clear();
            currentCharacters = 0;
        }

        for (var index = 0; index < lines.Count; index++)
        {
            var line = lines[index] ?? string.Empty;
            if (current.Count > 0 &&
                (current.Count >= lineLimit || currentCharacters + line.Length > characterLimit))
            {
                Flush();
                currentStart = index;
            }

            if (current.Count == 0)
                currentStart = index;
            current.Add(line);
            currentCharacters += line.Length;
        }

        Flush();
        return batches;
    }

    internal static bool ShouldClearDraft(string currentDraft, string sentDraft) =>
        string.Equals(currentDraft, sentDraft, StringComparison.Ordinal);

    internal static bool ShouldStartAutomaticListening(
        bool voiceEnabled,
        bool automaticallyStartListening,
        bool alreadyStarted,
        bool isClosed) =>
        voiceEnabled && automaticallyStartListening && !alreadyStarted && !isClosed;

    internal static CancellationTokenSource CreateManualAiRequestCancellation() => new();

    internal static AiRequest CreateScreenAiRequest(
        string prompt,
        IEnumerable<AiMessage> history,
        List<AiAttachment> attachments,
        IProgress<AiStreamDelta>? streamingProgress,
        IProgress<AiAgentEvent>? agentProgress=null,
        Func<AiInteractionRequest,CancellationToken,Task<AiInteractionResponse>>? interactionHandler=null) => new()
    {
        Prompt=prompt,
        History=[..history],
        Attachments=attachments,
        StreamingProgress=streamingProgress,
        AgentProgress=agentProgress,
        InteractionHandler=interactionHandler,
        ExpectStructuredResponse=true,
        MaxOutputTokens=4096
    };

    internal static bool CanAcceptAiUpdate(
        CancellationTokenSource? activeRequest,
        CancellationTokenSource candidate,
        bool isClosed,
        bool streamOpen = true) =>
        !isClosed &&
        streamOpen &&
        ReferenceEquals(activeRequest,candidate) &&
        !candidate.IsCancellationRequested;

    internal static bool ShouldFinalizeCanceledAiRequest(
        CancellationTokenSource? activeRequest,
        CancellationTokenSource candidate,
        bool isClosed) =>
        !isClosed &&
        ReferenceEquals(activeRequest,candidate) &&
        candidate.IsCancellationRequested;

    internal static void RestoreRecordingReference<T>(
        ISet<T> references,
        T item,
        bool wasReferenced)
        where T:notnull
    {
        if(wasReferenced)
            references.Add(item);
        else
            references.Remove(item);
    }

    internal static Rect GetPromptBarBounds(Rect monitor,double desiredHeight)
    {
        if(monitor.IsEmpty||!double.IsFinite(monitor.Width)||!double.IsFinite(monitor.Height)||monitor.Width<=0||monitor.Height<=0)return Rect.Empty;
        var width=Math.Min(PromptPreferredWidth,Math.Max(1,monitor.Width-PromptSideMargin*2));
        var availableHeight=Math.Max(1,monitor.Height-PromptTopMargin-PromptBottomMargin);
        var height=Math.Min(Math.Max(1,double.IsFinite(desiredHeight)?desiredHeight:1),availableHeight);
        return new Rect(
            monitor.Left+(monitor.Width-width)/2,
            Math.Max(monitor.Top+PromptTopMargin,monitor.Bottom-PromptBottomMargin-height),
            width,
            height);
    }

    /// <summary>
    /// Re-fits the composer after WPF has arranged its content.  A prompt bar
    /// can grow when reference chips or the first answer arrives, and the
    /// first measure pass may still report the previous height.  Keeping this
    /// clamp in the policy makes the final pass deterministic and guarantees
    /// that the rendered border (not just its desired size) stays on-screen.
    /// </summary>
    internal static Rect RefitPromptBarAfterArrange(Rect monitor, Rect candidate, double arrangedHeight)
    {
        if(monitor.IsEmpty||candidate.IsEmpty||!double.IsFinite(arrangedHeight)||arrangedHeight<=0)
            return candidate;

        var availableHeight=Math.Max(1,monitor.Height-PromptTopMargin-PromptBottomMargin);
        var height=Math.Min(arrangedHeight,availableHeight);
        var width=Math.Min(Math.Max(1,candidate.Width),Math.Max(1,monitor.Width-PromptSideMargin*2));
        var left=Math.Clamp(candidate.Left,monitor.Left+PromptSideMargin,Math.Max(monitor.Left+PromptSideMargin,monitor.Right-PromptSideMargin-width));
        var top=Math.Clamp(candidate.Top,monitor.Top+PromptTopMargin,Math.Max(monitor.Top+PromptTopMargin,monitor.Bottom-PromptBottomMargin-height));
        return new Rect(left,top,width,height);
    }

    internal static double GetPromptResponseMaxHeight(double promptHeight)=>
        !double.IsFinite(promptHeight)||promptHeight<=PromptReservedComposerHeight
            ?0
            :promptHeight-PromptReservedComposerHeight;

    internal static double GetAnswerViewportHeight(double monitorHeight)
    {
        if(!double.IsFinite(monitorHeight)||monitorHeight<=0)return 160;
        return Math.Clamp(monitorHeight*.34,160,300);
    }

    internal static bool ShouldAutoHidePromptBar(
        Point pointer,
        Rect promptBounds,
        Rect monitorBounds,
        IEnumerable<Rect> explicitSelections)
    {
        if(!promptBounds.IsEmpty&&promptBounds.Contains(pointer))return false;
        foreach(var selection in explicitSelections)
        {
            if(selection.IsEmpty||!selection.Contains(pointer))continue;
            if(!monitorBounds.IsEmpty&&CoversMostOfMonitor(selection,monitorBounds))return false;
            return true;
        }
        return false;
    }

    private static bool CoversMostOfMonitor(Rect selection,Rect monitor)
    {
        var intersection=Rect.Intersect(selection,monitor);
        if(intersection.IsEmpty||monitor.Width<=0||monitor.Height<=0)return false;
        return intersection.Width*intersection.Height/(monitor.Width*monitor.Height)>=.85;
    }

    internal static double ConstrainFloatingBarWidth(Rect monitor,double desiredWidth)
    {
        if(monitor.IsEmpty||!double.IsFinite(monitor.Width)||monitor.Width<=0)return 1;
        return Math.Min(Math.Max(1,double.IsFinite(desiredWidth)?desiredWidth:1),Math.Max(1,monitor.Width-16));
    }

    internal static bool IsPointerInFloatingBarInteractionZone(Point pointer,Rect barBounds,double transitionPadding)
    {
        if(barBounds.IsEmpty||!double.IsFinite(barBounds.Left)||!double.IsFinite(barBounds.Top)||
           !double.IsFinite(barBounds.Width)||!double.IsFinite(barBounds.Height)||barBounds.Width<=0||barBounds.Height<=0)
            return false;
        var padding=double.IsFinite(transitionPadding)?Math.Max(0,transitionPadding):0;
        barBounds.Inflate(padding,padding);
        return barBounds.Contains(pointer);
    }

    internal static bool HasContentGeometryChanged(Rect previous,Rect current)=>
        !AreClose(previous.Left,current.Left)||!AreClose(previous.Top,current.Top)||!AreClose(previous.Width,current.Width)||!AreClose(previous.Height,current.Height);

    private static bool AreClose(double left,double right)=>double.IsFinite(left)&&double.IsFinite(right)&&Math.Abs(left-right)<.01;

    internal static bool IsUsableSelection(double width,double height)=>
        double.IsFinite(width)&&double.IsFinite(height)&&width>=8&&height>=8;

    internal static bool CanRunImageOnlyCommand(bool isImplicit,string? videoPath)=>
        !isImplicit&&string.IsNullOrWhiteSpace(videoPath);

    internal static OverlayUndoTarget ResolveUndoTarget(
        bool editablePromptFocused,
        bool pointerOverPrompt,
        bool pointerOverExplicitSelection)=>
        editablePromptFocused&&pointerOverPrompt
            ?OverlayUndoTarget.Text
            :pointerOverExplicitSelection
                ?OverlayUndoTarget.Overlay
                :OverlayUndoTarget.Overlay;
}

public enum OverlayUndoTarget{Overlay,Text}

internal sealed record TranslationBatch(
    int StartIndex,
    IReadOnlyList<string> Lines,
    int MaxOutputTokens);
