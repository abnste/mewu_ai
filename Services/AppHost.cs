using System.Drawing;
using System.Windows;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Views;
using Forms=System.Windows.Forms;
namespace mewu_ai_Assistant.Services;
public sealed class AppHost : IDisposable
{
    private readonly System.Windows.Application _app; private readonly SingleInstanceService _single; private readonly SettingsService _settingsService=new();
    private GlobalHotkeyService? _hotkey; private Forms.NotifyIcon? _tray; private MainWindow? _main; private SettingsWindow? _settingsWindow;private int _captureActive;
    public AppSettings Settings { get; private set; } public bool IsExiting { get; private set; }
    public AppHost(System.Windows.Application app) { _app=app; Settings=_settingsService.Load(); _single=new(); }
    public bool Start()
    {
        if(!_single.IsPrimary){_single.SignalPrimary();return false;}
        _single.ActivationRequested+=()=>_app.Dispatcher.Invoke(ShowMainWindow);
        _main=new MainWindow(this); _app.MainWindow=_main;
        _hotkey=new GlobalHotkeyService(); _hotkey.Pressed+=BeginCapture; var hotkeyOk=_hotkey.Register(Settings.CaptureHotkey);
        new TempFileService().Cleanup(TimeSpan.FromDays(Math.Clamp(Settings.TempCleanupDays,1,30)));BuildTray();if(!hotkeyOk)Notify("快捷键注册失败，可能已被其他应用占用");return true;
    }
    private void BuildTray()
    {
        var menu=new Forms.ContextMenuStrip();
        menu.Items.Add("截图 / AI",null,(_,_)=>BeginCapture());menu.Items.Add("文字问答",null,(_,_)=>ShowTextAi()); menu.Items.Add("设置",null,(_,_)=>ShowSettings()); menu.Items.Add("打开主界面",null,(_,_)=>ShowMainWindow()); menu.Items.Add(new Forms.ToolStripSeparator()); menu.Items.Add("退出",null,(_,_)=>Exit());
        var iconPath=Path.Combine(AppContext.BaseDirectory,"MewuAI.ico");
        var trayIcon=File.Exists(iconPath)?new Icon(iconPath):SystemIcons.Application;
        _tray=new Forms.NotifyIcon { Text="喵呜AI",Icon=trayIcon,Visible=true,ContextMenuStrip=menu };
        _tray.MouseClick+=(_,e)=>{if(e.Button==Forms.MouseButtons.Left)BeginCapture();};
    }
    public void BeginCapture()=>_=BeginCaptureAsync();
    private async Task BeginCaptureAsync()
    {
        if(Interlocked.CompareExchange(ref _captureActive,1,0)!=0)return;
        try{if(Settings.CaptureDelaySeconds>0)await Task.Delay(TimeSpan.FromSeconds(Settings.CaptureDelaySeconds));await _app.Dispatcher.InvokeAsync(()=> { var overlay=new CaptureOverlayWindow(this);overlay.Closed+=(_,_)=>Interlocked.Exchange(ref _captureActive,0);overlay.Show(); overlay.Activate(); });}catch(Exception ex){Interlocked.Exchange(ref _captureActive,0);new PrivacyLogger().Error("Capture",ex);Notify("无法开始截图，请重试");}
    }
    public void ShowMainWindow() { _app.Dispatcher.Invoke(()=>{_main??=new MainWindow(this);_main.Show();_main.WindowState=WindowState.Normal;_main.Activate();}); }
    public void ShowSettings() { _app.Dispatcher.Invoke(()=>{ if(_settingsWindow is null){_settingsWindow=new SettingsWindow(this);_settingsWindow.Closed+=(_,_)=>_settingsWindow=null;} _settingsWindow.Show();_settingsWindow.Activate();}); }
    public void ShowTextAi(string initial="")=>_app.Dispatcher.Invoke(()=>new TextAiWindow(this,initial).Show());
    public bool TrySetCaptureHotkey(HotkeySetting hotkey){if(_hotkey?.Register(hotkey)==false){Notify("快捷键注册失败，旧快捷键仍然有效");return false;}Settings.CaptureHotkey=hotkey;return true;}
    public void SaveSettings(){_settingsService.Save(Settings);_main?.RefreshStatus();}
    public void Notify(string message){_tray?.ShowBalloonTip(1500,"喵呜AI",message,Forms.ToolTipIcon.Info);}
    public void Exit() { IsExiting=true; _tray!.Visible=false; _app.Shutdown(); }
    public void Dispose() { _tray?.Dispose(); _hotkey?.Dispose(); _single.Dispose(); new TempFileService().Cleanup(TimeSpan.Zero); }
}
