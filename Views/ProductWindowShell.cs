using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System.Windows.Media.Media3D;

namespace mewu_ai_Assistant.Views;

internal static class ProductWindowShell
{
    public static void Configure(Window window, string title, double width, double height, double minWidth, double minHeight, UIElement body, bool canResize = true)
    {
        window.Title = $"喵呜AI {title}";
        window.Width = width;
        window.Height = height;
        window.MinWidth = minWidth;
        window.MinHeight = minHeight;
        window.WindowStartupLocation = WindowStartupLocation.CenterScreen;
        window.WindowStyle = WindowStyle.None;
        window.ResizeMode = canResize ? ResizeMode.CanResize : ResizeMode.NoResize;
        window.AllowsTransparency = true;
        window.Background = Brushes.Transparent;
        window.Foreground = new SolidColorBrush(Color.FromRgb(36, 49, 72));
        window.UseLayoutRounding = true;
        window.SnapsToDevicePixels = true;
        TextOptions.SetTextFormattingMode(window, TextFormattingMode.Display);

        // Keep the chrome compact, but leave a predictable breathing space at
        // either side of the title.  The shell is inset below, so these values
        // are the content inset rather than a second shadow-sized margin.
        var titleBar = new Grid { Height = 46, Margin = new Thickness(18, 0, 10, 0), UseLayoutRounding = true, SnapsToDevicePixels = true };
        titleBar.ColumnDefinitions.Add(new ColumnDefinition());
        titleBar.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var titleContent = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
        titleContent.Children.Add(new Border
        {
            Width = 26,
            Height = 26,
            CornerRadius = new CornerRadius(8),
            Background = new SolidColorBrush(Color.FromRgb(232, 245, 255)),
            BorderBrush = new SolidColorBrush(Color.FromRgb(211, 235, 255)),
            BorderThickness = new Thickness(1),
            Padding = new Thickness(3),
            Child = new Image { Source = new System.Windows.Media.Imaging.BitmapImage(new Uri("pack://application:,,,/Assets/MewuAI.Icon.png")), Stretch = Stretch.Uniform }
        });
        titleContent.Children.Add(new TextBlock { Text = title, FontSize = 15, FontWeight = FontWeights.SemiBold, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(8, 0, 0, 0) });
        titleBar.Children.Add(titleContent);
        var close = new Button { Content = CloseIcon(), ToolTip = "关闭", Width = 32, Height = 32, Padding = new Thickness(0), Margin = new Thickness(0, 0, 0, 0) };
        System.Windows.Automation.AutomationProperties.SetName(close, "关闭窗口");
        close.SetResourceReference(FrameworkElement.StyleProperty, "IconButton");
        close.Click += (_, _) => window.Close();
        Grid.SetColumn(close, 1);
        titleBar.Children.Add(close);
        titleBar.MouseLeftButtonDown += (_, e) =>
        {
            // The close glyph is a Path inside the Button template, so
            // checking only OriginalSource would still start a drag when the
            // pointer lands on the glyph itself. Walk the visual/logical
            // ancestors and keep every button interaction out of DragMove.
            if (IsInsideButton(e.OriginalSource)) return;
            if (e.ClickCount == 2 && canResize) window.WindowState = window.WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;
            else window.DragMove();
        };

        var layout = new Grid();
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        layout.RowDefinitions.Add(new RowDefinition());
        Grid.SetRow(body, 1);
        layout.Children.Add(titleBar);
        layout.Children.Add(body);
        // A transparent Window needs a real, full-size backdrop.  The old
        // single border used a ten-DIP margin to make room for its shadow;
        // that left a visible transparent strip through which the main window
        // showed whenever this small window was centred above it.  The outer
        // rounded backdrop now fills the client area and masks that strip,
        // while the inset surface keeps a small ring for a restrained inner
        // shadow.  Only the four rounded corner pixels remain transparent.
        var shellBackground = new SolidColorBrush(Color.FromRgb(247, 249, 253));
        var shellBorder = new SolidColorBrush(Color.FromRgb(215, 225, 238));
        var backdrop = new Border
        {
            Background = shellBackground,
            BorderBrush = shellBorder,
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(18),
            ClipToBounds = true,
            SnapsToDevicePixels = true
        };
        var surface = new Border
        {
            Margin = new Thickness(4),
            Background = shellBackground,
            BorderBrush = shellBorder,
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(15),
            ClipToBounds = true,
            SnapsToDevicePixels = true,
            Child = layout,
            // The outer backdrop masks the transparent edge; this effect is
            // intentionally modest so the ring reads as depth instead of a
            // broad grey halo on light surfaces.
            Effect = new DropShadowEffect
            {
                Color = Color.FromRgb(60, 73, 92),
                BlurRadius = 16,
                ShadowDepth = 2,
                Opacity = .18,
                RenderingBias = RenderingBias.Quality
            }
        };
        var shell = new Grid
        {
            Background = Brushes.Transparent,
            UseLayoutRounding = true,
            SnapsToDevicePixels = true
        };
        shell.Children.Add(backdrop);
        shell.Children.Add(surface);
        window.Content = shell;
    }

    private static bool IsInsideButton(object? source)
    {
        var current = source as DependencyObject;
        while (current is not null)
        {
            if (current is ButtonBase) return true;
            current = current switch
            {
                Visual or Visual3D => VisualTreeHelper.GetParent(current),
                FrameworkContentElement content => content.Parent,
                _ => LogicalTreeHelper.GetParent(current)
            };
        }
        return false;
    }

    private static System.Windows.Shapes.Path CloseIcon() => new()
    {
        Width = 15,
        Height = 15,
        Stretch = Stretch.Uniform,
        Stroke = new SolidColorBrush(Color.FromRgb(82, 99, 122)),
        StrokeThickness = 1.8,
        StrokeStartLineCap = PenLineCap.Round,
        StrokeEndLineCap = PenLineCap.Round,
        Data = Geometry.Parse("M3,3 L13,13 M13,3 L3,13")
    };
}
