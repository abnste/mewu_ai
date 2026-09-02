using System.Drawing;
using System.Drawing.Drawing2D;
using System.Windows;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Views;
using Forms=System.Windows.Forms;
namespace mewu_ai_Assistant.Services;
public sealed class AppHost : IDisposable
{
    internal static readonly TimeSpan TempMediaShutdownWait=TimeSpan.FromSeconds(5);
    private readonly System.Windows.Application _app; private readonly SingleInstanceService _single; private readonly CancellationTokenSource _lifetime=new(); private readonly StartupActivationGate _activationGate=new();
    private readonly AiProviderFactory _aiProviderFactory=new();
    private readonly HermesRuntimeService _hermesRuntime;
    private readonly HermesReadAloudService _hermesReadAloud;
    private SettingsService? _settingsService;
    private GlobalHotkeyService? _hotkey; private Forms.NotifyIcon? _tray; private Forms.ContextMenuStrip? _trayMenu; private Forms.ToolStripMenuItem? _captureMenuItem; private Icon? _ownedTrayIcon; private Font? _ownedTrayMenuFont; private MainWindow? _main; private SettingsWindow? _settingsWindow; private readonly List<Window> _auxiliaryWindows=[]; private bool _restoreMainAfterAuxiliary; private int _captureActive;
    private int _disposed;
    public AppSettings Settings { get; private set; }=new(); public bool IsExiting { get; private set; }
    public bool IsCaptureActive => Volatile.Read(ref _captureActive) != 0;
    public AppHost(System.Windows.Application app)
    {
        _app=app??throw new ArgumentNullException(nameof(app));
        _hermesRuntime=new HermesRuntimeService();
        _hermesReadAloud=new HermesReadAloudService(_app.Dispatcher);
        _single=new();
        if(_single.IsPrimary)_single.ActivationRequested+=()=>_activationGate.Signal(QueueMainWindowActivation);
    }
    public bool Start()
    {
        if(!_single.IsPrimary){_single.SignalPrimary();return false;}
        CrashDiagnosticsService.InitializePrimary();
        CrashDiagnosticsService.MarkOperation("加载设置");
        _settingsService=new();Settings=_settingsService.Load();
        _main=new MainWindow(this); _app.MainWindow=_main;
        _hotkey=new GlobalHotkeyService(); _hotkey.Pressed+=BeginCapture; var hotkeyOk=_hotkey.Register(Settings.CaptureHotkey);
        var retention=TimeSpan.FromDays(Math.Clamp(Settings.TempCleanupDays,1,30));new TempFileService().Cleanup(retention);ClipboardService.CleanupStagedFiles(retention);BuildTray();if(!hotkeyOk)Notify("快捷键注册失败，可能已被其他应用占用");_activationGate.MarkStarted(QueueMainWindowActivation);CrashDiagnosticsService.MarkOperation("空闲");return true;
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
            Renderer=new LightTrayMenuRenderer()
        };
        _trayMenu=menu;
        _captureMenuItem=AddTrayMenuItem(menu,"截图 / AI",(_,_)=>BeginCapture());
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
        RefreshAiEntryVisibility();
    }
    private static Forms.ToolStripMenuItem AddTrayMenuItem(Forms.ContextMenuStrip menu,string text,EventHandler onClick)
    {
        var item=new Forms.ToolStripMenuItem(text){AutoSize=false,Height=36,Margin=new Forms.Padding(0,1,0,1),Padding=new Forms.Padding(12,0,14,0),TextAlign=ContentAlignment.MiddleLeft};item.Click+=onClick;menu.Items.Add(item);return item;
    }
    public void BeginCapture(){if(!IsExiting&&Volatile.Read(ref _disposed)==0)_=BeginCaptureAsync();}
    private async Task BeginCaptureAsync()
    {
        if(Interlocked.CompareExchange(ref _captureActive,1,0)!=0)return;
        CrashDiagnosticsService.MarkOperation("启动屏幕助手");
        var token=_lifetime.Token;
        try
        {
            // A capture can be triggered from the tray or the global hotkey
            // while the launcher is still visible.  Hide it before the frame
            // is frozen so the assistant never captures its own launcher and
            // the overlay remains the single, clean surface the user sees.
            await _app.Dispatcher.InvokeAsync(() =>
            {
                if (_main?.IsVisible == true) _main.Hide();
            });
            if(Settings.CaptureDelaySeconds>0)await Task.Delay(TimeSpan.FromSeconds(Settings.CaptureDelaySeconds),token);
            token.ThrowIfCancellationRequested();
            await _app.Dispatcher.InvokeAsync(()=>
            {
                token.ThrowIfCancellationRequested();
                var overlay=new CaptureOverlayWindow(this);overlay.Closed+=(_,_)=>{Interlocked.Exchange(ref _captureActive,0);CrashDiagnosticsService.MarkOperation("空闲");};overlay.Show();overlay.Activate();
            });
        }
        catch(OperationCanceledException) when(token.IsCancellationRequested){Interlocked.Exchange(ref _captureActive,0);CrashDiagnosticsService.MarkOperation("空闲");}
        catch(Exception ex)
        {
            Interlocked.Exchange(ref _captureActive,0);new PrivacyLogger().Error("Capture",ex);
            CrashDiagnosticsService.MarkOperation("截图启动失败后空闲");
            if(!token.IsCancellationRequested&&!IsExiting)try{Notify("无法开始截图，请重试");}catch{}
        }
    }
    public void ShowMainWindow() { _app.Dispatcher.Invoke(()=>{_main??=new MainWindow(this);_main.Show();_main.WindowState=WindowState.Normal;_main.Activate();}); }
    public void ShowSettings() { _app.Dispatcher.Invoke(()=>{ if(_settingsWindow is null){_settingsWindow=new SettingsWindow(this);var window=_settingsWindow;window.Closed+=(_,_)=>{if(ReferenceEquals(_settingsWindow,window))_settingsWindow=null;FinishAuxiliary(window);};} PrepareAuxiliary(_settingsWindow);_settingsWindow.Show();_settingsWindow.WindowState=WindowState.Normal;_settingsWindow.Activate();}); }
    public HermesInstallation? DiscoverHermes()=>_hermesRuntime.Discover();

    public Task<IReadOnlyList<HermesAgentOption>> GetHermesAgentOptionsAsync(CancellationToken cancellationToken)
        =>_hermesRuntime.GetAgentOptionsAsync(cancellationToken);

    public Task<IReadOnlyList<HermesModelOption>> GetHermesModelOptionsAsync(string? profile,bool refresh,CancellationToken cancellationToken)
        =>_hermesRuntime.GetModelOptionsAsync(profile,refresh,cancellationToken);

    public Task<bool> TestHermesConnectionAsync(string? profile,CancellationToken cancellationToken)
        =>_hermesRuntime.TestConnectionAsync(profile,cancellationToken);

    /// <summary>
    /// Selects the only permitted backend for a conversation. Once local
    /// Hermes is enabled, configuration or connection failures remain Hermes
    /// failures and can never silently leak the prompt to a remote Provider.
    /// </summary>
    public IAiProvider? CreateConversationProvider(HermesConversationKind kind,out string? error)
    {
        error=null;
        if(Volatile.Read(ref _disposed)!=0||IsExiting)
        {
            error="喵呜AI 正在退出，无法开始新的对话。";
            return null;
        }
        return CreateConversationProviderCore(kind,()=>Settings,_hermesRuntime,_aiProviderFactory,out error);
    }

    internal static IAiProvider? CreateConversationProviderCore(
        HermesConversationKind kind,
        Func<AppSettings> settingsAccessor,
        HermesRuntimeService hermesRuntime,
        AiProviderFactory aiProviderFactory,
        out string? error)
    {
        ArgumentNullException.ThrowIfNull(settingsAccessor);
        ArgumentNullException.ThrowIfNull(hermesRuntime);
        ArgumentNullException.ThrowIfNull(aiProviderFactory);
        error=null;
        if(!Enum.IsDefined(kind))
        {
            error="Hermes 会话类型无效。";
            return null;
        }
        var settings=settingsAccessor();
        if(!settings.HermesEnabled)return aiProviderFactory.Create(settings,out error);
        try
        {
            // This runtime and provider live for the whole AppHost lifetime.
            // Model/reasoning changes are read from Settings on the next turn
            // without replacing the persistent Hermes session.
            return hermesRuntime.GetConversationProvider(kind,settingsAccessor);
        }
        catch(Exception ex)when(ex is InvalidOperationException or ArgumentException or ObjectDisposedException)
        {
            error=$"本机 Hermes 不可用：{ex.Message}";
            try{new PrivacyLogger().Error("HermesConversationRoute",ex);}catch{}
            return null;
        }
    }

    /// <summary>Translation remains a strict remote-Provider operation.</summary>
    public IAiProvider? CreateTranslationProvider(out string? error)=>_aiProviderFactory.Create(Settings,out error);

    public bool IsTranslationAvailable(out string? error)
    {
        var provider=_aiProviderFactory.Create(Settings,out error);
        return provider is not null;
    }

    public bool IsConversationAvailable(out string? error)
    {
        error=null;
        if(Settings.HermesEnabled)
        {
            if(_hermesRuntime.Discover() is not null)return true;
            error="已启用本机 Hermes，但未找到可用的 Windows Hermes 安装。";
            return false;
        }

        return IsTranslationAvailable(out error);
    }

    public bool IsScreenAiAvailable(out string? error)
    {
        error=null;
        if(Settings.HermesEnabled)return IsConversationAvailable(out error);
        var provider=_aiProviderFactory.Create(Settings,out error);
        if(provider is null)return false;
        if(provider.Capabilities.SupportsImage)return true;
        error="当前默认模型只支持文字，请选择支持图片理解的多模态模型";
        return false;
    }

    public Task ReadHermesResponseAloudAsync(string text,CancellationToken cancellationToken=default)
    {
        if(!Settings.HermesEnabled||!Settings.HermesAutoReadAloud||string.IsNullOrWhiteSpace(text))return Task.CompletedTask;
        if(Volatile.Read(ref _disposed)!=0||IsExiting)return Task.CompletedTask;
        return _hermesReadAloud.SpeakAsync(_hermesRuntime,text,Settings.HermesProfile,cancellationToken);
    }

    public void StopHermesReadAloud()=>_hermesReadAloud.Stop();

    /// <summary>
    /// Presents one auxiliary surface at a time. Keeping the launcher and
    /// other editor surfaces hidden while a child is open prevents transparent
    /// rounded shells from stacking over one another; nested transitions (for
    /// example, opening Settings from the capture overlay) restore the previous
    /// surface when the new one closes.
    /// </summary>
    private void PrepareAuxiliary(Window window)
    {
        var existingIndex=_auxiliaryWindows.LastIndexOf(window);
        if(existingIndex>=0)
        {
            if(existingIndex==_auxiliaryWindows.Count-1)return;
            // The same window may be suspended underneath another auxiliary
            // surface. Move it to the front instead of adding a duplicate
            // stack entry that could otherwise be restored twice.
            _auxiliaryWindows.RemoveAt(existingIndex);
        }
        if(_auxiliaryWindows.Count==0)_restoreMainAfterAuxiliary=_main?.IsVisible==true;
        else
        {
            var current=_auxiliaryWindows[^1];
            try{if(current.IsVisible)current.Hide();}catch(Exception ex){try{new PrivacyLogger().Error("AuxiliaryHide",ex);}catch{}}
        }
        if(!ReferenceEquals(_main,window))try{_main?.Hide();}catch(Exception ex){try{new PrivacyLogger().Error("MainHide",ex);}catch{}}
        if(!ReferenceEquals(_settingsWindow,window))try{_settingsWindow?.Hide();}catch(Exception ex){try{new PrivacyLogger().Error("SettingsHide",ex);}catch{}}
        _auxiliaryWindows.Add(window);
    }

    private void FinishAuxiliary(Window window)
    {
        var index=_auxiliaryWindows.LastIndexOf(window);if(index<0)return;
        var wasTop=index==_auxiliaryWindows.Count-1;_auxiliaryWindows.RemoveAt(index);if(!wasTop)return;
        while(_auxiliaryWindows.Count>0)
        {
            var previous=_auxiliaryWindows[^1];
            try
            {
                if(previous.IsVisible){previous.Activate();return;}
                previous.Show();previous.WindowState=WindowState.Normal;previous.Activate();return;
            }
            catch(Exception ex){_auxiliaryWindows.RemoveAt(_auxiliaryWindows.Count-1);try{new PrivacyLogger().Error("AuxiliaryRestore",ex);}catch{}}
        }
        if(_restoreMainAfterAuxiliary&&!IsExiting)ShowMainWindow();
        _restoreMainAfterAuxiliary=false;
    }
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
        if((previous.HermesEnabled&&!candidate.HermesEnabled)||(previous.HermesAutoReadAloud&&!candidate.HermesAutoReadAloud))
            _hermesReadAloud.Stop();
        try{_main?.RefreshStatus();}
        catch(Exception ex){try{new PrivacyLogger().Error("SettingsUiRefresh",ex);}catch{}warning??="设置已保存，但主界面状态刷新失败。";}
        RefreshAiEntryVisibility();return true;
    }
    private void RefreshAiEntryVisibility(){var screenAiAvailable=IsScreenAiAvailable(out _);if(_captureMenuItem is not null)_captureMenuItem.Text=screenAiAvailable?"截图 / AI":"截图";}
    public void Notify(string message){_tray?.ShowBalloonTip(1500,"喵呜AI",message,Forms.ToolTipIcon.Info);}
    public void Exit() { CrashDiagnosticsService.MarkOperation("正在退出");IsExiting=true;_lifetime.Cancel();if(_tray is not null)_tray.Visible=false;_app.Shutdown(); }
    public void Dispose()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        var shouldCleanupTemp=_single.IsPrimary;
        IsExiting=true;_lifetime.Cancel();Interlocked.Exchange(ref _captureActive,0);
        // Drain speech first, then stop the runtime and only afterwards clean
        // temporary media. This prevents deleting an audio file still opened
        // by WPF or disposing Hermes while synthesis is still in flight.
        DisposeSafely(_hermesReadAloud,"HermesReadAloudDispose");
        DisposeSafely(_hermesRuntime,"HermesRuntimeDispose");
        try{if(_tray is not null)_tray.Visible=false;}catch{}
        DisposeSafely(_tray,"TrayDispose");DisposeSafely(_trayMenu,"TrayMenuDispose");DisposeSafely(_ownedTrayMenuFont,"TrayMenuFontDispose");DisposeSafely(_ownedTrayIcon,"TrayIconDispose");DisposeSafely(_hotkey,"HotkeyDispose");
        if(shouldCleanupTemp)try
        {
            var released=TempMediaRegistry.Shared.WaitForNoActiveLeases(TempMediaShutdownWait);
            var cleanup=new TempFileService().Cleanup(TimeSpan.Zero);
            if(!released&&cleanup.SkippedLeasedCount>0)new PrivacyLogger().Error("TempCleanupOnExit",new TimeoutException($"等待临时媒体释放超时，已保留 {cleanup.SkippedLeasedCount} 个仍在使用的文件"));
        }
        catch(Exception ex){try{new PrivacyLogger().Error("TempCleanupOnExit",ex);}catch{}}
        CrashDiagnosticsService.MarkCleanExit();
        DisposeSafely(_single,"SingleInstanceDispose");
        _lifetime.Dispose();
    }
    private static void DisposeSafely(IDisposable? resource,string component){try{resource?.Dispose();}catch(Exception ex){try{new PrivacyLogger().Error(component,ex);}catch{}}}

    private sealed class LightTrayMenuRenderer : Forms.ToolStripProfessionalRenderer
    {
        private static readonly Color Background=Color.FromArgb(250,251,253);
        private static readonly Color Border=Color.FromArgb(215,222,233);
        private static readonly Color Hover=Color.FromArgb(237,242,250);

        internal LightTrayMenuRenderer():base(new LightTrayMenuColorTable()){RoundedEdges=true;}

        protected override void OnRenderToolStripBackground(Forms.ToolStripRenderEventArgs e)
        {
            if(e.ToolStrip is not Forms.ContextMenuStrip){base.OnRenderToolStripBackground(e);return;}
            e.Graphics.SmoothingMode=SmoothingMode.AntiAlias;
            var bounds=new Rectangle(0,0,e.ToolStrip.Width-1,e.ToolStrip.Height-1);
            using var path=RoundedRectangle(bounds,11);
            using var brush=new SolidBrush(Background);
            e.Graphics.FillPath(brush,path);
        }

        protected override void OnRenderToolStripBorder(Forms.ToolStripRenderEventArgs e)
        {
            if(e.ToolStrip is not Forms.ContextMenuStrip){base.OnRenderToolStripBorder(e);return;}
            e.Graphics.SmoothingMode=SmoothingMode.AntiAlias;
            var bounds=new Rectangle(0,0,e.ToolStrip.Width-1,e.ToolStrip.Height-1);
            using var path=RoundedRectangle(bounds,11);
            using var pen=new Pen(Border);
            e.Graphics.DrawPath(pen,path);
        }

        protected override void OnRenderMenuItemBackground(Forms.ToolStripItemRenderEventArgs e)
        {
            if(!e.Item.Selected)return;
            e.Graphics.SmoothingMode=SmoothingMode.AntiAlias;
            // ToolStrip paints each item in an item-local graphics context. Using
            // Item.Bounds here applies the parent offset a second time, so most of
            // the hover pill is clipped and only a patch behind the text survives.
            var bounds=TrayMenuRenderLayout.GetHoverBounds(e.Item.Size);
            if(bounds.IsEmpty)return;
            using var path=RoundedRectangle(bounds,7);
            using var brush=new SolidBrush(Hover);
            using var pen=new Pen(Color.FromArgb(205,216,232));
            e.Graphics.FillPath(brush,path);e.Graphics.DrawPath(pen,path);
        }

        private static GraphicsPath RoundedRectangle(Rectangle bounds,int radius)
        {
            var path=new GraphicsPath();var diameter=radius*2;var arc=new Rectangle(bounds.Left,bounds.Top,diameter,diameter);
            path.AddArc(arc,180,90);arc.X=bounds.Right-diameter;path.AddArc(arc,270,90);arc.Y=bounds.Bottom-diameter;path.AddArc(arc,0,90);arc.X=bounds.Left;path.AddArc(arc,90,90);path.CloseFigure();return path;
        }
    }

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

internal static class TrayMenuRenderLayout
{
    internal static Rectangle GetHoverBounds(System.Drawing.Size itemSize)
    {
        const int horizontalInset=2;
        const int verticalInset=1;
        var width=itemSize.Width-horizontalInset*2;
        var height=itemSize.Height-verticalInset*2;
        return width>0&&height>0
            ?new Rectangle(horizontalInset,verticalInset,width,height)
            :Rectangle.Empty;
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
