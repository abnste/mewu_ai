using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Effects;

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

        var titleBar = new Grid { Height = 50, Margin = new Thickness(18, 0, 12, 0) };
        titleBar.ColumnDefinitions.Add(new ColumnDefinition());
        titleBar.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        titleBar.Children.Add(new TextBlock { Text = title, FontSize = 15, FontWeight = FontWeights.SemiBold, VerticalAlignment = VerticalAlignment.Center });
        var close = new Button { Content = "×", ToolTip = "关闭", Width = 34, Height = 34, Padding = new Thickness(0), FontSize = 18 };
        close.SetResourceReference(FrameworkElement.StyleProperty, "IconButton");
        close.Click += (_, _) => window.Close();
        Grid.SetColumn(close, 1);
        titleBar.Children.Add(close);
        titleBar.MouseLeftButtonDown += (_, e) =>
        {
            if (e.OriginalSource is Button) return;
            if (e.ClickCount == 2 && canResize) window.WindowState = window.WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;
            else window.DragMove();
        };

        var layout = new Grid();
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        layout.RowDefinitions.Add(new RowDefinition());
        Grid.SetRow(body, 1);
        layout.Children.Add(titleBar);
        layout.Children.Add(body);
        window.Content = new Border
        {
            Margin = new Thickness(12),
            Background = new SolidColorBrush(Color.FromRgb(247, 249, 253)),
            BorderBrush = new SolidColorBrush(Color.FromRgb(215, 225, 238)),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(20),
            ClipToBounds = true,
            Child = layout,
            Effect = new DropShadowEffect { Color = Color.FromRgb(60, 73, 92), BlurRadius = 32, ShadowDepth = 8, Opacity = .25 }
        };
    }
}
