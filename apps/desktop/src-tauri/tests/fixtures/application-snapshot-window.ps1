# SPDX-License-Identifier: MPL-2.0
# Owned synthetic WPF source for an explicitly selected integration test.
param([Parameter(Mandatory=$true)][string]$ReadyPath,[Parameter(Mandatory=$true)][string]$ClosePath)
$ErrorActionPreference='Stop'
if([Console]::In.ReadLine()-cne 'start'){exit 2}
Add-Type -AssemblyName PresentationFramework
Add-Type -AssemblyName PresentationCore
$taskWindow=[Windows.Window]::new()
$taskWindow.Title='Mewu synthetic snapshot source'
$taskWindow.Width=400
$taskWindow.Height=360
$taskWindow.WindowStyle='None'
$taskWindow.ResizeMode='NoResize'
$taskWindow.ShowActivated=$false
$taskWindow.ShowInTaskbar=$false
$taskWindow.Background=[Windows.Media.Brushes]::White
$taskScroll=[Windows.Controls.ScrollViewer]::new()
$taskScroll.VerticalScrollBarVisibility='Auto'
$taskScroll.HorizontalScrollBarVisibility='Disabled'
$taskStack=[Windows.Controls.StackPanel]::new()
for($taskRow=0;$taskRow -lt 50;$taskRow++){
    $taskLine=[Windows.Controls.Border]::new()
    $taskLine.Height=30
    $taskLine.Background=[Windows.Media.SolidColorBrush]::new([Windows.Media.Color]::FromRgb((40+$taskRow*31%190),(40+$taskRow*47%190),(40+$taskRow*73%190)))
    $taskText=[Windows.Controls.TextBlock]::new()
    $taskText.Text=('Mewu synthetic row {0:D3} / {1}' -f $taskRow,($taskRow*7919))
    $taskText.Foreground=[Windows.Media.Brushes]::White
    $taskText.FontSize=16
    $taskText.Margin=[Windows.Thickness]::new(12,3,0,0)
    $taskLine.Child=$taskText
    $null=$taskStack.Children.Add($taskLine)
}
$taskScroll.Content=$taskStack
$taskWindow.Content=$taskScroll
$taskWindow.add_ContentRendered({
    $taskScroll.ScrollToVerticalOffset(300)
    $taskWindow.UpdateLayout()
    $taskHandle=[Windows.Interop.WindowInteropHelper]::new($taskWindow).Handle
    $taskReady=[ordered]@{handle=$taskHandle.ToInt64();processId=$PID}
    [IO.File]::WriteAllText($ReadyPath,($taskReady|ConvertTo-Json -Compress),[Text.UTF8Encoding]::new($false))
})
$taskTimer=[Windows.Threading.DispatcherTimer]::new()
$taskTimer.Interval=[TimeSpan]::FromMilliseconds(100)
$taskTimer.add_Tick({if([IO.File]::Exists($ClosePath)){$taskTimer.Stop();$taskWindow.Close()}})
$taskWindow.add_Closed({[Windows.Threading.Dispatcher]::CurrentDispatcher.BeginInvokeShutdown('Normal')})
$taskTimer.Start()
$taskWindow.Show()
[Windows.Threading.Dispatcher]::Run()
