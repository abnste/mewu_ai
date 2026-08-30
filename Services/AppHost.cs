using System.Drawing;
using System.Windows;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Views;
using Forms=System.Windows.Forms;
namespace mewu_ai_Assistant.Services;
public sealed class AppHost : IDisposable
{
    internal static readonly TimeSpan TempMediaShutdownWait=TimeSpan.FromSeconds(5);
    private readonly System.Windows.Application _app; private readonly SingleInstanceService _single; private readonly CancellationTokenSource _lifetime=new(); private readonly StartupActivationGate _activationGate=new();
    private SettingsService? _settingsService;
    private GlobalHotkeyService? _hotkey; private Forms.NotifyIcon? _tray; private Forms.ContextMenuStrip? _trayMenu; private Icon? _ownedTrayIcon; private Font? _ownedTrayMenuFont; private MainWindow? _main; private SettingsWindow? _settingsWindow;private int _captureActive;
    private int _disposed;
    public AppSettings Settings { get; private set; }=new(); public bool IsExiting { get; private set; }
    public bool IsCaptureActive => Volatile.Read(ref _captureActive) != 0;
    public AppHost(System.Windows.Application app)
    {
        _app=app;
        _single=new();
        if(_single.IsPrimary)_single.ActivationRequested+=()=>_activationGate.Signal(QueueMainWindowActivation);
    }
    public bool Start()
    {
        if(!_single.IsPrimary){_single.SignalPrimary();return false;}
        _settingsService=new();Settings=_settingsService.Load();
        _main=new MainWindow(this); _app.MainWindow=_main;
        _hotkey=new GlobalHotkeyService(); _hotkey.Pressed+=BeginCapture; var hotkeyOk=_hotkey.Register(Settings.CaptureHotkey);
        var retention=TimeSpan.FromDays(Math.Clamp(Settings.TempCleanupDays,1,30));new TempFileService().Cleanup(retention);ClipboardService.CleanupStagedFiles(retention);BuildTray();if(!hotkeyOk)Notify("快捷键注册失败，可能已被其他应用占用");_activationGate.MarkStarted(QueueMainWindowActivation);return true;
    }
    private void QueueMainWindowActivation()
    {
        if(IsExiting||Volatile.Read(ref _disposed)!=0)return;
        _app.Dispatcher.BeginInvoke(new Action(ShowMainWindow));
    }
    private void BuildTray()
    {
        var menu=new Forms.ContextMenuStrip
        {
            BackColor=Color.FromArgb(250,251,253),
            ForeColor=Color.FromArgb(38,49,66),
            Font=_ownedTrayMenuFont=new Font("Microsoft YaHei UI",9F,System.Drawing.FontStyle.Regular,GraphicsUnit.Point),
            Padding=new Forms.Padding(6),
            ShowCheckMargin=false,
            ShowImageMargin=false,
            MinimumSize=new System.Drawing.Size(156,0),
            Renderer=new Forms.ToolStripProfessionalRenderer(new LightTrayMenuColorTable()) { RoundedEdges=true }
        };
        _trayMenu=menu;
        AddTrayMenuItem(menu,"截图 / AI",(_,_)=>BeginCapture());
        AddTrayMenuItem(menu,"文字问答",(_,_)=>ShowTextAi());
        AddTrayMenuItem(menu,"设置",(_,_)=>ShowSettings());
        AddTrayMenuItem(menu,"打开主界面",(_,_)=>ShowMainWindow());
        menu.Items.Add(new Forms.ToolStripSeparator{Margin=new Forms.Padding(8,4,8,4)});
        AddTrayMenuItem(menu,"退出",(_,_)=>Exit());
        Icon? trayIcon=null;
        try
        {
            if(Environment.ProcessPath is {Length:>0} executable&&File.Exists(executable))
                trayIcon=_ownedTrayIcon=Icon.ExtractAssociatedIcon(executable);
        }
        catch(Exception ex){try{new PrivacyLogger().Error("TrayIcon",ex);}catch{}}
        trayIcon??=SystemIcons.Application;
        _tray=new Forms.NotifyIcon { Text="喵呜AI",Icon=trayIcon,Visible=true,ContextMenuStrip=menu };
        _tray.MouseClick+=(_,e)=>{if(e.Button==Forms.MouseButtons.Left)BeginCapture();};
    }
    private static void AddTrayMenuItem(Forms.ContextMenuStrip menu,string text,EventHandler onClick)
    {
        var item=new Forms.ToolStripMenuItem(text){AutoSize=true,Margin=new Forms.Padding(0,1,0,1),Padding=new Forms.Padding(10,6,18,6)};item.Click+=onClick;menu.Items.Add(item);
    }
    public void BeginCapture(){if(!IsExiting&&Volatile.Read(ref _disposed)==0)_=BeginCaptureAsync();}
    private async Task BeginCaptureAsync()
    {
        if(Interlocked.CompareExchange(ref _captureActive,1,0)!=0)return;
        var token=_lifetime.Token;
        try
        {
            if(Settings.CaptureDelaySeconds>0)await Task.Delay(TimeSpan.FromSeconds(Settings.CaptureDelaySeconds),token);
            token.ThrowIfCancellationRequested();
            await _app.Dispatcher.InvokeAsync(()=>
            {
                token.ThrowIfCancellationRequested();
                var overlay=new CaptureOverlayWindow(this);overlay.Closed+=(_,_)=>Interlocked.Exchange(ref _captureActive,0);overlay.Show();overlay.Activate();
            });
        }
        catch(OperationCanceledException) when(token.IsCancellationRequested){Interlocked.Exchange(ref _captureActive,0);}
        catch(Exception ex)
        {
            Interlocked.Exchange(ref _captureActive,0);new PrivacyLogger().Error("Capture",ex);
            if(!token.IsCancellationRequested&&!IsExiting)try{Notify("无法开始截图，请重试");}catch{}
        }
    }
    public void ShowMainWindow() { _app.Dispatcher.Invoke(()=>{_main??=new MainWindow(this);_main.Show();_main.WindowState=WindowState.Normal;_main.Activate();}); }
    public void ShowSettings() { _app.Dispatcher.Invoke(()=>{ if(_settingsWindow is null){_settingsWindow=new SettingsWindow(this);_settingsWindow.Closed+=(_,_)=>_settingsWindow=null;} _settingsWindow.Show();_settingsWindow.Activate();}); }
    public void ShowTextAi(string initial="")=>_app.Dispatcher.Invoke(()=>new TextAiWindow(this,initial).Show());
    public bool TryApplySettings(AppSettings candidate,out string? error,out string? warning)
    {
        error=null;warning=null;var previous=Settings;var startupChanged=candidate.LaunchAtStartup!=previous.LaunchAtStartup;
        var hotkeyChanged=candidate.CaptureHotkey.Key!=previous.CaptureHotkey.Key||candidate.CaptureHotkey.Modifiers!=previous.CaptureHotkey.Modifiers;
        if(_hotkey?.Register(candidate.CaptureHotkey)==false){error="该快捷键可能已被其他应用占用，旧快捷键仍然有效。";return false;}
        try
        {
            if(startupChanged)StartupService.SetEnabled(candidate.LaunchAtStartup);
            (_settingsService??throw new InvalidOperationException("设置服务尚未初始化")).Save(candidate);
        }
        catch(Exception ex)
        {
            if(hotkeyChanged&&_hotkey?.Register(previous.CaptureHotkey)==false)
            {
                try{new PrivacyLogger().Error("HotkeySettingsRollback",new InvalidOperationException("设置保存失败后无法恢复旧快捷键"));}catch{}
            }
            if(startupChanged)try{StartupService.SetEnabled(previous.LaunchAtStartup);}catch(Exception rollbackError){try{new PrivacyLogger().Error("StartupSettingsRollback",rollbackError);}catch{}}
            error=ex.Message;return false;
        }
        Settings=candidate;
        try{_main?.RefreshStatus();}
        catch(Exception ex){try{new PrivacyLogger().Error("SettingsUiRefresh",ex);}catch{}warning??="设置已保存，但主界面状态刷新失败。";}
        return true;
    }
    public void Notify(string message){_tray?.ShowBalloonTip(1500,"喵呜AI",message,Forms.ToolTipIcon.Info);}
    public void Exit() { IsExiting=true;_lifetime.Cancel();if(_tray is not null)_tray.Visible=false;_app.Shutdown(); }
    public void Dispose()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        var shouldCleanupTemp=_single.IsPrimary;
        IsExiting=true;_lifetime.Cancel();Interlocked.Exchange(ref _captureActive,0);
        try{if(_tray is not null)_tray.Visible=false;}catch{}
        DisposeSafely(_tray,"TrayDispose");DisposeSafely(_trayMenu,"TrayMenuDispose");DisposeSafely(_ownedTrayMenuFont,"TrayMenuFontDispose");DisposeSafely(_ownedTrayIcon,"TrayIconDispose");DisposeSafely(_hotkey,"HotkeyDispose");
        if(shouldCleanupTemp)try
        {
            var released=TempMediaRegistry.Shared.WaitForNoActiveLeases(TempMediaShutdownWait);
            var cleanup=new TempFileService().Cleanup(TimeSpan.Zero);
            if(!released&&cleanup.SkippedLeasedCount>0)new PrivacyLogger().Error("TempCleanupOnExit",new TimeoutException($"等待临时媒体释放超时，已保留 {cleanup.SkippedLeasedCount} 个仍在使用的文件"));
        }
        catch(Exception ex){try{new PrivacyLogger().Error("TempCleanupOnExit",ex);}catch{}}
        DisposeSafely(_single,"SingleInstanceDispose");
        _lifetime.Dispose();
    }
    private static void DisposeSafely(IDisposable? resource,string component){try{resource?.Dispose();}catch(Exception ex){try{new PrivacyLogger().Error(component,ex);}catch{}}}

    private sealed class LightTrayMenuColorTable : Forms.ProfessionalColorTable
    {
        private static readonly Color Background=Color.FromArgb(250,251,253);
        private static readonly Color Hover=Color.FromArgb(237,242,250);
        private static readonly Color Border=Color.FromArgb(215,222,233);
        internal LightTrayMenuColorTable()=>UseSystemColors=false;
        public override Color ToolStripDropDownBackground=>Background;
        public override Color ImageMarginGradientBegin=>Background;
        public override Color ImageMarginGradientMiddle=>Background;
        public override Color ImageMarginGradientEnd=>Background;
        public override Color MenuBorder=>Border;
        public override Color MenuItemBorder=>Color.FromArgb(205,216,232);
        public override Color MenuItemSelected=>Hover;
        public override Color MenuItemSelectedGradientBegin=>Hover;
        public override Color MenuItemSelectedGradientEnd=>Hover;
        public override Color SeparatorDark=>Color.FromArgb(226,231,239);
        public override Color SeparatorLight=>Background;
    }
}

internal sealed class StartupActivationGate
{
    private int _started;
    private int _pending;

    internal void Signal(Action activate)
    {
        ArgumentNullException.ThrowIfNull(activate);
        Interlocked.Exchange(ref _pending,1);
        if(Volatile.Read(ref _started)!=0)Drain(activate);
    }

    internal void MarkStarted(Action activate)
    {
        ArgumentNullException.ThrowIfNull(activate);
        Volatile.Write(ref _started,1);
        Drain(activate);
    }

    private void Drain(Action activate)
    {
        if(Interlocked.Exchange(ref _pending,0)!=0)activate();
    }
}
