using System.Windows.Threading;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private bool _selectionCleanupQueued;

    private void QueueSelectionResourceCleanup()
    {
        if (_selectionCleanupQueued || _closed) return;
        _selectionCleanupQueued = true;
        _ = Dispatcher.BeginInvoke(DispatcherPriority.ContextIdle, new Action(() =>
        {
            _selectionCleanupQueued = false;
            // Async operations may still hold a local rollback snapshot. Their
            // completion paths schedule another pass after dropping ownership.
            if (_closed || _request is not null || _overlayRequest is not null ||
                _recordingMode || _recordingCountdownActive || _longCaptureMode || _drawingMode ||
                _selecting || _moving) return;

            var retained = new HashSet<SelectionItem>(_selections);
            foreach (var snapshot in _overlayHistory.RetainedStates) RetainSnapshot(snapshot);
            RetainSnapshot(_pointerOperationBefore);
            RetainSnapshot(_resizeOperationBefore);
            RetainSnapshot(_drawingOperationBefore);
            RetainSnapshot(_longCaptureBefore);
            retained.UnionWith(_lastSentSelections);
            if (_recordingItem is not null) retained.Add(_recordingItem);
            if (_longCaptureItem is not null) retained.Add(_longCaptureItem);

            foreach (var item in _ownedSelections.Where(item => !retained.Contains(item)).ToArray())
            {
                ReleaseSelectionResources(item);
                item.Image.Source = null;
                item.CapturedImageOverride = null;
                item.AnnotationOcrDocument = null;
                item.TextLayer = NoTextLayerState.Instance;
                item.Markup.Strokes.Clear();
                item.DrawingElements.Clear();
                item.DrawingOrder.Clear();
                item.DrawingRedo.Clear();
                item.AnnotationNotes.Clear();
                item.TextOverlays.Children.Clear();
                item.AiAnnotations.Children.Clear();
                _ownedSelections.Remove(item);
            }

            void RetainSnapshot(OverlaySnapshot? snapshot)
            {
                if (snapshot is null) return;
                foreach (var state in snapshot.Selections) retained.Add(state.Item);
                retained.UnionWith(snapshot.LastSentSelections);
            }
        }));
    }
}
