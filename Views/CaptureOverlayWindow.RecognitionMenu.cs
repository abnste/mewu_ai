// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using mewu_ai_Assistant.Interop;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private void RecognitionMenuOpened(object sender, RoutedEventArgs e)
    {
        _historyCopyMenuOpen = true;
        var available = !_closed && _overlayRequest is null && Active is { IsImplicit: false, VideoPath: null };
        MemoryFillMenuItem.Header = L("扫描填充", "Scan and fill");
        MemoryFillMenuItem.IsEnabled = available;
        if (sender is ContextMenu menu && PresentationSource.FromVisual(menu) is HwndSource source)
            NativeMethods.ApplyOwnedWindowCaptureVisibility(source.Handle, new WindowInteropHelper(this).Handle);
    }

    private void RecognitionMenuClosed(object sender, RoutedEventArgs e) => _historyCopyMenuOpen = false;
}
