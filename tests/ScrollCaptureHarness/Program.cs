using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Threading;

// A local, non-sensitive manual QA surface. It contains no automation or
// network access; use real mouse input to test the product capture overlay.
internal static class Program
{
    [STAThread]
    private static void Main()
    {
        var app=new Application();
        var header=new TextBlock{Text="Scroll capture QA — colored cards / smooth wheel",FontSize=20,Margin=new Thickness(18)};
        var rows=new StackPanel();
        for(var i=0;i<80;i++)
        {
            var brush=new SolidColorBrush(Color.FromRgb((byte)(50+i*73%180),(byte)(50+i*47%180),(byte)(50+i*109%180)));
            var content=new StackPanel();
            content.Children.Add(new TextBlock{Text=$"Card {i+1:000} — {new[]{"Mountain","Ocean","Forest","River","Desert"}[i%5]}",FontSize=24,FontWeight=FontWeights.Bold});
            content.Children.Add(new Border{Height=50,Width=80+i*37%450,Background=brush,HorizontalAlignment=HorizontalAlignment.Left,Margin=new Thickness(0,10,0,10)});
            content.Children.Add(new TextBlock{Text=$"Reference {(i+13)*7919} — independent image detail {i*3571:X}",FontSize=18});
            rows.Children.Add(new Border{Height=180,Margin=new Thickness(15,8,15,8),Padding=new Thickness(20),BorderThickness=new Thickness(14,0,0,0),BorderBrush=brush,Background=Brushes.White,Child=content});
        }
        var scroll=new ScrollViewer{Content=rows,VerticalScrollBarVisibility=ScrollBarVisibility.Visible,Background=new SolidColorBrush(Color.FromRgb(230,237,244))};
        var root=new DockPanel();DockPanel.SetDock(header,Dock.Top);root.Children.Add(header);root.Children.Add(scroll);
        var window=new Window{Title="Mewu Scroll Capture QA",Width=1050,Height=740,Left=180,Top=25,Content=root};
        var timer=new DispatcherTimer{Interval=TimeSpan.FromMilliseconds(16)};var clock=new Stopwatch();double from=0,to=0;
        timer.Tick+=(_,_)=>{var t=Math.Min(1,clock.Elapsed.TotalMilliseconds/260);scroll.ScrollToVerticalOffset(from+(to-from)*t);if(t>=1)timer.Stop();};
        scroll.PreviewMouseWheel+=(_,e)=>{e.Handled=true;from=scroll.VerticalOffset;to=Math.Clamp((timer.IsEnabled?to:from)-e.Delta*2,0,scroll.ScrollableHeight);clock.Restart();timer.Start();};
        scroll.ScrollChanged+=(_,_)=>header.Text=$"Scroll capture QA — offset {scroll.VerticalOffset:F0}px / smooth wheel";
        window.Loaded+=(_,_)=>scroll.ScrollToVerticalOffset(600);
        app.Run(window);
    }
}
