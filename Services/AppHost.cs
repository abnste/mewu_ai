using System.Drawing;
using System.Windows;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Views;
using Forms=System.Windows.Forms;
namespace mewu_ai_Assistant.Services;
public sealed class AppHost : IDisposable
{
    private readonly System.Windows.Application _app; private readonly SingleInstanceService _single; private readonly SettingsService _settingsService=new();
    private GlobalHotkeyService? _hotkey; private Forms.NotifyIcon? _tray; private MainWindow? _main; private SettingsWindow? _settingsWindow;
    public AppSettings Settings { get; private set; } public bool IsExiting { get; private set; }
    public AppHost(System.Windows.Application app) { _app=app; Settings=_settingsService.Load(); _single=new(); }
    public bool Start()
    {
        if(!_single.IsPrimary){_single.SignalPrimary();return false;}
        _single.ActivationRequested+=()=>_app.Dispatcher.Invoke(ShowMainWindow);
        _main=new MainWindow(this); _app.MainWindow=_main;
        _hotkey=new GlobalHotkeyService(); _hotkey.Pressed+=BeginCapture; _hotkey.Register(Settings.CaptureHotkey);
        new TempFileService().Cleanup(TimeSpan.FromDays(3));BuildTray(); return true;
    }
    private void BuildTray()
    {
        var menu=new Forms.ContextMenuStrip();
        menu.Items.Add("截图 / AI",null,(_,_)=>BeginCapture()); menu.Items.Add("设置",null,(_,_)=>ShowSettings()); menu.Items.Add("打开主界面",null,(_,_)=>ShowMainWindow()); menu.Items.Add(new Forms.ToolStripSeparator()); menu.Items.Add("退出",null,(_,_)=>Exit());
        _tray=new Forms.NotifyIcon { Text="喵呜AI",Icon=SystemIcons.Application,Visible=true,ContextMenuStrip=menu };
        _tray.MouseClick+=(_,e)=>{if(e.Button==Forms.MouseButtons.Left)BeginCapture();};
    }
    public async void BeginCapture()
    {
        if(Settings.CaptureDelaySeconds>0)await Task.Delay(TimeSpan.FromSeconds(Settings.CaptureDelaySeconds));
        _app.Dispatcher.Invoke(()=> { var overlay=new CaptureOverlayWindow(this); overlay.Show(); overlay.Activate(); });
    }
    public void ShowMainWindow() { _app.Dispatcher.Invoke(()=>{_main??=new MainWindow(this);_main.Show();_main.WindowState=WindowState.Normal;_main.Activate();}); }
    public void ShowSettings() { _app.Dispatcher.Invoke(()=>{ if(_settingsWindow is null){_settingsWindow=new SettingsWindow(this);_settingsWindow.Closed+=(_,_)=>_settingsWindow=null;} _settingsWindow.Show();_settingsWindow.Activate();}); }
    public void SaveSettings() { _settingsService.Save(Settings); _hotkey?.Register(Settings.CaptureHotkey); }
    public void Exit() { IsExiting=true; _tray!.Visible=false; _app.Shutdown(); }
    public void Dispose() { _tray?.Dispose(); _hotkey?.Dispose(); _single.Dispose(); }
}
