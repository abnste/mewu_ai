using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Models;
using System.Windows;
using Xunit;

namespace MewuAI.Tests;

public sealed class CaptureOverlayPolicyTests
{
    [Theory]
    [InlineData(true,true,false,OverlayUndoTarget.Text)]
    [InlineData(true,true,true,OverlayUndoTarget.Text)]
    [InlineData(true,false,true,OverlayUndoTarget.Overlay)]
    [InlineData(false,true,false,OverlayUndoTarget.Overlay)]
    public void ResolveUndoTarget_UsesPointerBeforeStalePromptFocus(
        bool promptFocused,bool pointerOverPrompt,bool pointerOverSelection,OverlayUndoTarget expected)
    {
        Assert.Equal(expected,CaptureOverlayPolicy.ResolveUndoTarget(promptFocused,pointerOverPrompt,pointerOverSelection));
    }

    [Theory]
    [InlineData(false, null, true)]
    [InlineData(false, "", true)]
    [InlineData(true, null, false)]
    [InlineData(false, "C:\\Temp\\capture.mp4", false)]
    public void ImageOnlyCommandsRejectImplicitAndVideoSelections(bool isImplicit,string? videoPath,bool expected)
    {
        Assert.Equal(expected,CaptureOverlayPolicy.CanRunImageOnlyCommand(isImplicit,videoPath));
    }

    [Fact]
    public void SelectSendTargets_ExcludesImplicitSelectionWhenExplicitSelectionsExist()
    {
        var implicitSelection = new Target("implicit", true, false);
        var first = new Target("first", false, false);
        var second = new Target("second", false, false);

        var selected = CaptureOverlayPolicy.SelectSendTargets(
            new[] { implicitSelection, first, second },
            item => item.IsImplicit,
            item => item.IsReferenced);

        Assert.Equal(new[] { first, second }, selected);
    }

    [Fact]
    public void SelectSendTargets_KeepsImplicitSelectionWhenItIsTheOnlyChoice()
    {
        var implicitSelection = new Target("implicit", true, false);

        var selected = CaptureOverlayPolicy.SelectSendTargets(
            new[] { implicitSelection },
            item => item.IsImplicit,
            item => item.IsReferenced);

        Assert.Equal(new[] { implicitSelection }, selected);
    }

    [Fact]
    public void SelectSendTargets_UsesOnlyReferencedExplicitSelections()
    {
        var implicitReference = new Target("implicit", true, true);
        var first = new Target("first", false, false);
        var second = new Target("second", false, true);

        var selected = CaptureOverlayPolicy.SelectSendTargets(
            new[] { implicitReference, first, second },
            item => item.IsImplicit,
            item => item.IsReferenced);

        Assert.Equal(new[] { second }, selected);
    }

    [Fact]
    public void SpatialAnnotationTargetsKeepFullAttachmentIndexesAndExcludeVideos()
    {
        var attachments=new[]{new Attachment("video",true),new Attachment("image-1",false),new Attachment("image-2",false)};

        var targets=CaptureOverlayPolicy.SelectSpatialAnnotationTargets(attachments,item=>item.IsVideo);

        Assert.Equal(new[]{1,2},targets.Select(target=>target.RegionIndex));
        Assert.Equal(new[]{attachments[1],attachments[2]},targets.Select(target=>target.Item));
    }

    [Fact]
    public void CreateTranslationBatches_SplitsLargeDocumentsAndPreservesOrder()
    {
        var lines = Enumerable.Range(0, 121).Select(index => $"line-{index:D3}").ToArray();

        var batches = CaptureOverlayPolicy.CreateTranslationBatches(lines);

        Assert.True(batches.Count > 1);
        Assert.All(batches, batch => Assert.InRange(batch.Lines.Count, 1, CaptureOverlayPolicy.TranslationBatchLineLimit));
        Assert.Equal(lines, batches.SelectMany(batch => batch.Lines));
        Assert.Equal(Enumerable.Range(0, batches.Count).Select(index => batches.Take(index).Sum(batch => batch.Lines.Count)), batches.Select(batch => batch.StartIndex));
    }

    [Fact]
    public void CreateTranslationBatches_IsolatesASingleOversizedLine()
    {
        var oversized = new string('长', 4000);

        var batches = CaptureOverlayPolicy.CreateTranslationBatches(new[] { "before", oversized, "after" });

        var oversizedBatch = Assert.Single(batches, batch => batch.Lines.Contains(oversized));
        Assert.Equal(new[] { oversized }, oversizedBatch.Lines);
        Assert.Equal(new[] { "before", oversized, "after" }, batches.SelectMany(batch => batch.Lines));
    }

    [Fact]
    public void CreateTranslationBatches_ScalesAndCapsOutputTokenBudget()
    {
        var shortBatch = Assert.Single(CaptureOverlayPolicy.CreateTranslationBatches(new[] { "short" }, characterLimit: int.MaxValue));
        var mediumBatch = Assert.Single(CaptureOverlayPolicy.CreateTranslationBatches(new[] { new string('中', 1500) }, characterLimit: int.MaxValue));
        var longBatch = Assert.Single(CaptureOverlayPolicy.CreateTranslationBatches(new[] { new string('长', 10000) }, characterLimit: int.MaxValue));

        Assert.True(mediumBatch.MaxOutputTokens > shortBatch.MaxOutputTokens);
        Assert.Equal(4096, longBatch.MaxOutputTokens);
    }

    [Theory]
    [InlineData("原始问题", "原始问题", true)]
    [InlineData("原始问题 ", "原始问题", false)]
    [InlineData("用户已继续输入", "原始问题", false)]
    public void ShouldClearDraft_RequiresAnExactMatch(string currentDraft, string sentDraft, bool expected)
    {
        Assert.Equal(expected, CaptureOverlayPolicy.ShouldClearDraft(currentDraft, sentDraft));
    }

    [Theory]
    [InlineData(true, true, false, false, true)]
    [InlineData(false, true, false, false, false)]
    [InlineData(true, false, false, false, false)]
    [InlineData(true, true, true, false, false)]
    [InlineData(true, true, false, true, false)]
    public void AutomaticListening_RequiresAnEnabledFirstOpenPrompt(
        bool voiceEnabled,
        bool automaticallyStartListening,
        bool alreadyStarted,
        bool isClosed,
        bool expected)
    {
        Assert.Equal(expected, CaptureOverlayPolicy.ShouldStartAutomaticListening(
            voiceEnabled,
            automaticallyStartListening,
            alreadyStarted,
            isClosed));
    }

    [Fact]
    public async Task AiRequestCancellation_RemainsManualWhileProviderOwnsTheDeadline()
    {
        using var cancellation=CaptureOverlayPolicy.CreateManualAiRequestCancellation();
        await Task.Delay(TimeSpan.FromMilliseconds(30),TestContext.Current.CancellationToken);
        Assert.False(cancellation.IsCancellationRequested);
        cancellation.Cancel();
        Assert.True(cancellation.IsCancellationRequested);
    }

    [Fact]
    public void ScreenAiRequestAlwaysRequiresStructuredAnnotations()
    {
        var attachment=new AiAttachment(AiAttachmentType.Image,"image/png",[1,2,3],ProviderOwnsData:false);
        var request=CaptureOverlayPolicy.CreateScreenAiRequest(
            "标出重点",
            [new AiMessage("system","返回结构化批注")],
            [attachment],
            null);

        Assert.True(request.ExpectStructuredResponse);
        Assert.Equal(4096,request.MaxOutputTokens);
        Assert.Same(attachment,Assert.Single(request.Attachments));
        Assert.Equal("system",Assert.Single(request.History).Role);
    }

    [Fact]
    public void AiUpdatesAreRejectedAfterCancellationReplacementClosureOrStreamCompletion()
    {
        using var request=new CancellationTokenSource();using var replacement=new CancellationTokenSource();
        Assert.True(CaptureOverlayPolicy.CanAcceptAiUpdate(request,request,false));
        Assert.False(CaptureOverlayPolicy.CanAcceptAiUpdate(replacement,request,false));
        Assert.False(CaptureOverlayPolicy.CanAcceptAiUpdate(request,request,true));
        Assert.False(CaptureOverlayPolicy.CanAcceptAiUpdate(request,request,false,false));
        request.Cancel();
        Assert.False(CaptureOverlayPolicy.CanAcceptAiUpdate(request,request,false));
    }

    [Fact]
    public void CanceledAiRequestIsFinalizedOnlyByItsLiveOwningOverlay()
    {
        using var request=new CancellationTokenSource();using var replacement=new CancellationTokenSource();
        request.Cancel();
        Assert.True(CaptureOverlayPolicy.ShouldFinalizeCanceledAiRequest(request,request,false));
        Assert.False(CaptureOverlayPolicy.ShouldFinalizeCanceledAiRequest(replacement,request,false));
        Assert.False(CaptureOverlayPolicy.ShouldFinalizeCanceledAiRequest(request,request,true));
    }

    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public void FailedRecordingRestoresItsOriginalReferenceState(bool wasReferenced)
    {
        var item=new object();
        var references=new HashSet<object>();
        if(!wasReferenced)references.Add(item);

        CaptureOverlayPolicy.RestoreRecordingReference(references,item,wasReferenced);

        Assert.Equal(wasReferenced,references.Contains(item));
    }

    [Fact]
    public void RecordingStopWatchdogIsFiniteAndUserRecoverable()
    {
        Assert.InRange(CaptureOverlayPolicy.RecordingStopTimeout,TimeSpan.FromSeconds(1),TimeSpan.FromSeconds(30));
    }

    [Fact]
    public void PromptBar_IsCenteredInsideTheSelectedNegativeCoordinateMonitor()
    {
        var monitor=new Rect(-1280,0,1280,720);
        var result=CaptureOverlayPolicy.GetPromptBarBounds(monitor,120);
        Assert.Equal(574,result.Width);
        Assert.Equal(-927,result.Left);
        Assert.Equal(576,result.Top);
        Assert.True(monitor.Contains(result));
    }

    [Fact]
    public void PromptBar_ShrinksInsteadOfOverflowingANarrowMonitor()
    {
        var monitor=new Rect(800,50,300,500);
        var result=CaptureOverlayPolicy.GetPromptBarBounds(monitor,180);
        Assert.Equal(268,result.Width);
        Assert.Equal(816,result.Left);
        Assert.Equal(346,result.Top);
        Assert.True(monitor.Contains(result));
    }

    [Fact]
    public void PromptBar_RefitKeepsArrangedChipLayoutInsideMonitor()
    {
        var monitor=new Rect(-1920,0,1920,1080);
        var candidate=new Rect(-1300,1010,574,96);

        var result=CaptureOverlayPolicy.RefitPromptBarAfterArrange(monitor,candidate,164);

        Assert.Equal(574,result.Width);
        Assert.Equal(892,result.Top);
        Assert.Equal(164,result.Height);
        Assert.True(monitor.Contains(result));
    }

    [Fact]
    public void PromptBar_RefitCapsOversizedArrangedLayoutToUsableHeight()
    {
        var monitor=new Rect(0,0,640,360);
        var result=CaptureOverlayPolicy.RefitPromptBarAfterArrange(monitor,new Rect(0,0,574,500),500);

        Assert.Equal(320,result.Height);
        Assert.Equal(16,result.Top);
        Assert.True(monitor.Contains(result));
    }

    [Theory]
    [InlineData(500,368)]
    [InlineData(132,0)]
    [InlineData(80,0)]
    public void PromptResponseAreaLeavesComposerVisible(double promptHeight,double expected)
    {
        Assert.Equal(expected,CaptureOverlayPolicy.GetPromptResponseMaxHeight(promptHeight));
    }

    [Theory]
    [InlineData(360,160)]
    [InlineData(720,244.8)]
    [InlineData(1200,300)]
    public void AnswerViewportRemainsReadableAndBounded(double monitorHeight,double expected)
    {
        Assert.Equal(expected,CaptureOverlayPolicy.GetAnswerViewportHeight(monitorHeight),5);
    }

    [Fact]
    public void PromptHoverWinsOverAnUnderlyingSelection()
    {
        var pointer=new Point(300,650);
        var prompt=new Rect(100,600,400,100);
        var monitor=new Rect(0,0,800,720);

        Assert.False(CaptureOverlayPolicy.ShouldAutoHidePromptBar(pointer,prompt,monitor,[new Rect(0,0,800,720)]));
    }

    [Fact]
    public void NearlyFullScreenSelectionCannotPermanentlyHidePromptBar()
    {
        var monitor=new Rect(0,0,1920,1080);

        Assert.False(CaptureOverlayPolicy.ShouldAutoHidePromptBar(new Point(900,500),Rect.Empty,monitor,[new Rect(0,0,1920,1080)]));
        Assert.True(CaptureOverlayPolicy.ShouldAutoHidePromptBar(new Point(300,250),Rect.Empty,monitor,[new Rect(200,150,500,300)]));
    }

    [Fact]
    public void FloatingToolbarWrapsToTheMonitorWidthInsteadOfOverflowing()
    {
        var monitor=new Rect(800,50,300,500);
        Assert.Equal(284,CaptureOverlayPolicy.ConstrainFloatingBarWidth(monitor,600));
        Assert.Equal(120,CaptureOverlayPolicy.ConstrainFloatingBarWidth(monitor,120));
    }

    [Fact]
    public void FloatingToolbarInteractionZoneIncludesItsPointerTransitGap()
    {
        var toolbar=new Rect(500,508,420,52);

        Assert.True(CaptureOverlayPolicy.IsPointerInFloatingBarInteractionZone(new Point(700,503),toolbar,10));
        Assert.True(CaptureOverlayPolicy.IsPointerInFloatingBarInteractionZone(new Point(700,530),toolbar,10));
        Assert.False(CaptureOverlayPolicy.IsPointerInFloatingBarInteractionZone(new Point(700,490),toolbar,10));
        Assert.False(CaptureOverlayPolicy.IsPointerInFloatingBarInteractionZone(new Point(480,530),toolbar,10));
    }

    [Fact]
    public void ContentLayersAreInvalidatedByMoveAndResizeButNotLayoutNoise()
    {
        var original=new Rect(10,20,300,200);
        Assert.False(CaptureOverlayPolicy.HasContentGeometryChanged(original,new Rect(10.005,20,300,200)));
        Assert.True(CaptureOverlayPolicy.HasContentGeometryChanged(original,new Rect(11,20,300,200)));
        Assert.True(CaptureOverlayPolicy.HasContentGeometryChanged(original,new Rect(10,20,301,200)));
    }

    [Theory]
    [InlineData(7.9,200,false)]
    [InlineData(200,7.9,false)]
    [InlineData(8,8,true)]
    [InlineData(double.NaN,100,false)]
    public void InterruptedSelection_OnlyKeepsFiniteUsableRegions(double width,double height,bool expected)
    {
        Assert.Equal(expected,CaptureOverlayPolicy.IsUsableSelection(width,height));
    }

    private sealed record Target(string Id, bool IsImplicit, bool IsReferenced);
    private sealed record Attachment(string Id,bool IsVideo);
}
