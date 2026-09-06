// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Globalization;
using System.Windows;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Views;
using Forms=System.Windows.Forms;
namespace mewu_ai_Assistant.Services;
public sealed class AppHost : IDisposable
{
    internal static readonly TimeSpan TempMediaShutdownWait=TimeSpan.FromSeconds(5);
    private readonly System.Windows.Application _app; private readonly SingleInstanceService _single; private readonly CancellationTokenSource _lifetime=new(); private readonly StartupActivationGate _activationGate=new(); private readonly CultureInfo? _uiCultureOverride;
    private readonly AiProviderFactory _aiProviderFactory=new();
    private readonly HermesRuntimeService _hermesRuntime;
    private readonly HermesReadAloudService _hermesReadAloud;
    private SettingsService? _settingsService;
    // Conversation history is also retained for the lifetime of this host.
    // This makes closing and reopening the capture overlay lossless even
    // when the user deliberately leaves disk history disabled. Entries are
    // filtered by provider/model before they are exposed to another overlay.
    private readonly object _sessionHistoryGate=new();
    private readonly List<ConversationHistoryEntry> _sessionConversationHistory=[];
    private GlobalHotkeyService? _hotkey; private Forms.NotifyIcon? _tray; private Forms.ContextMenuStrip? _trayMenu; private Icon? _ownedTrayIcon; private Font? _ownedTrayMenuFont; private MainWindow? _main; private SettingsWindow? _settingsWindow; private readonly List<Window> _auxiliaryWindows=[]; private bool _restoreMainAfterAuxiliary; private int _captureActive;
    private int _disposed;
    public AppSettings Settings { get; private set; }=new(); public bool IsExiting { get; private set; }
    public bool IsCaptureActive => Volatile.Read(ref _captureActive) != 0;
    public AppHost(System.Windows.Application app,CultureInfo? uiCultureOverride=null)
    {
        _app=app??throw new ArgumentNullException(nameof(app));
        _uiCultureOverride=uiCultureOverride;
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
        LocalizationService.Initialize(_uiCultureOverride is null?Settings.UiLanguage:"system",_uiCultureOverride??CultureInfo.CurrentUICulture);
        _main=CreateMainWindow(); _app.MainWindow=_main;
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
            Font=_ownedTrayMenuFont=new Font(LocalizationService.IsEnglish?"Segoe UI":"Microsoft YaHei UI",9F,System.Drawing.FontStyle.Regular,GraphicsUnit.Point),
            Padding=new Forms.Padding(6),
            ShowCheckMargin=false,
            ShowImageMargin=false,
            MinimumSize=new System.Drawing.Size(196,0),
            AutoSize=true,
            Renderer=new LightTrayMenuRenderer()
        };
        _trayMenu=menu;
        AddTrayMenuItem(menu,LocalizationService.T("设置","Settings"),(_,_)=>ShowSettings());
        AddTrayMenuItem(menu,LocalizationService.T("打开主界面","Open MewuAI"),(_,_)=>ShowMainWindow());
        menu.Items.Add(new Forms.ToolStripSeparator{Margin=new Forms.Padding(8,4,8,4)});
        AddTrayMenuItem(menu,LocalizationService.T("退出","Quit"),(_,_)=>Exit());
        Icon? trayIcon=null;
        try
        {
            if(Environment.ProcessPath is {Length:>0} executable&&File.Exists(executable))
                trayIcon=_ownedTrayIcon=Icon.ExtractAssociatedIcon(executable);
        }
        catch(Exception ex){try{new PrivacyLogger().Error("TrayIcon",ex);}catch{}}
        trayIcon??=SystemIcons.Application;
        _tray=new Forms.NotifyIcon { Text="MewuAI",Icon=trayIcon,Visible=true,ContextMenuStrip=menu };
        _tray.MouseClick+=(_,e)=>{if(e.Button==Forms.MouseButtons.Left)BeginCapture();};
    }
    private static Forms.ToolStripMenuItem AddTrayMenuItem(Forms.ContextMenuStrip menu,string text,EventHandler onClick)
    {
        var item=new Forms.ToolStripMenuItem(text){AutoSize=false,Width=Math.Max(156,menu.MinimumSize.Width-menu.Padding.Horizontal),Height=36,Margin=new Forms.Padding(0,1,0,1),Padding=new Forms.Padding(12,0,14,0),TextAlign=ContentAlignment.MiddleLeft};item.Click+=onClick;menu.Items.Add(item);return item;
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
    private MainWindow CreateMainWindow()
    {
        var window=new MainWindow(this);
        window.Closed+=(_,_)=>BeginShutdown();
        return window;
    }
    public void ShowMainWindow() { if(IsExiting||_app.Dispatcher.HasShutdownStarted)return;_app.Dispatcher.Invoke(()=>{if(IsExiting)return;_main??=CreateMainWindow();_main.Show();_main.WindowState=WindowState.Normal;_main.Activate();}); }
    public void ShowSettings(bool showAi=false) { _app.Dispatcher.Invoke(()=>{ if(_settingsWindow is null){_settingsWindow=new SettingsWindow(this);var window=_settingsWindow;window.Closed+=(_,_)=>{if(ReferenceEquals(_settingsWindow,window))_settingsWindow=null;FinishAuxiliary(window);};} if(showAi)_settingsWindow.ShowAiPage();PrepareAuxiliary(_settingsWindow);_settingsWindow.Show();_settingsWindow.WindowState=WindowState.Normal;_settingsWindow.Activate();}); }
    public HermesInstallation? DiscoverHermes()=>_hermesRuntime.Discover();

    public Task<IReadOnlyList<HermesAgentOption>> GetHermesAgentOptionsAsync(CancellationToken cancellationToken)
        =>_hermesRuntime.GetAgentOptionsAsync(cancellationToken);

    public Task<IReadOnlyList<HermesModelOption>> GetHermesModelOptionsAsync(string? profile,bool refresh,CancellationToken cancellationToken)
        =>_hermesRuntime.GetModelOptionsAsync(profile,refresh,cancellationToken);

    public Task<bool> TestHermesConnectionAsync(string? profile,CancellationToken cancellationToken)
        =>_hermesRuntime.TestConnectionAsync(profile,cancellationToken);

    /// <summary>Creates the provider chosen for the current conversation.</summary>
    public IAiProvider? CreateConversationProvider(HermesConversationKind kind,out string? error)
        =>CreateConversationProvider(kind,Settings.ConversationChannelId,out error);

    public IAiProvider? CreateConversationProvider(HermesConversationKind kind,string? channelId,out string? error)
    {
        error=null;
        if(Volatile.Read(ref _disposed)!=0||IsExiting)
        {
            error="喵呜AI 正在退出，无法开始新的对话。";
            return null;
        }
        return CreateConversationProviderCore(kind,()=>Settings,_hermesRuntime,_aiProviderFactory,channelId,out error);
    }

    internal IReadOnlyList<ConversationChannel> GetConversationChannels()
    {
        var channels=new List<ConversationChannel>();
        foreach(var provider in Settings.Providers ?? [])
        {
            if(provider is null)continue;
            var api=_aiProviderFactory.Create(Settings,provider.Id,out _);
            if(api is null)continue;
            channels.Add(new($"api:{provider.Id}",BuildApiChannelName(provider),provider.Id,provider.Model,ConversationChannelKind.Api,api.Capabilities.SupportsImage,api.Capabilities.SupportsVideo));
        }
        if(Settings.HermesEnabled&&_hermesRuntime.Discover() is not null)
        {
            if(!string.IsNullOrWhiteSpace(Settings.HermesProfile)&&!string.IsNullOrWhiteSpace(Settings.HermesModel))
                channels.Add(new("hermes",$"Hermes · {Settings.HermesProfile}","hermes",Settings.HermesModel??string.Empty,ConversationChannelKind.Hermes,true,true));
        }
        if(Settings.CodexEnabled&&CodexAppServer.Discover() is not null)
        {
            try{CodexSettingsPolicy.Validate(Settings);channels.Add(new("codex-work",$"ChatGPT Work · {Settings.CodexModel}","codex-work",Settings.CodexModel,ConversationChannelKind.Codex,Settings.CodexSupportsImage,Settings.CodexSupportsImage));}catch(InvalidOperationException){}
        }
        if(Settings.WorkBuddyEnabled&&WorkBuddyAcpServer.Discover() is not null)
        {
            try{WorkBuddySettingsPolicy.Validate(Settings.WorkBuddyModel,Settings.WorkBuddyReasoningEffort);channels.Add(new("workbuddy",$"WorkBuddy · {Settings.WorkBuddyModel}","workbuddy",Settings.WorkBuddyModel,ConversationChannelKind.WorkBuddy,Settings.WorkBuddySupportsImage,Settings.WorkBuddySupportsImage));}catch(InvalidOperationException){}
        }
        if(Settings.MiniMaxCodeEnabled&&MiniMaxCodeRuntime.TryGetDesktopSession() is not null)
        {
            try
            {
                MiniMaxCodeRuntime.ValidateModel(Settings.MiniMaxCodeModel);
                var model=MiniMaxCodeRuntime.KnownModels.First(item=>item.Model.Equals(Settings.MiniMaxCodeModel,StringComparison.OrdinalIgnoreCase));
                channels.Add(new("minimax-code",$"MiniMax Code · {model.Name}","minimax-code",model.Model,ConversationChannelKind.MiniMaxCode,model.SupportsVision,model.SupportsVision));
            }
            catch(InvalidOperationException){}
        }
        return channels;
    }

    private static string BuildApiChannelName(AiProviderSettings provider)
        =>string.IsNullOrWhiteSpace(provider.Name)?$"API · {provider.Model}":$"API · {provider.Name} · {provider.Model}";

    internal static IAiProvider? CreateConversationProviderCore(
        HermesConversationKind kind,
        Func<AppSettings> settingsAccessor,
        HermesRuntimeService hermesRuntime,
        AiProviderFactory aiProviderFactory,
        out string? error)
        =>CreateConversationProviderCore(kind,settingsAccessor,hermesRuntime,aiProviderFactory,null,out error);

    internal static IAiProvider? CreateConversationProviderCore(
        HermesConversationKind kind,
        Func<AppSettings> settingsAccessor,
        HermesRuntimeService hermesRuntime,
        AiProviderFactory aiProviderFactory,
        string? channelId,
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
        channelId=string.IsNullOrWhiteSpace(channelId)?null:channelId.Trim();
        if(channelId is not null&&channelId.StartsWith("api:",StringComparison.Ordinal))
            return aiProviderFactory.Create(settings,channelId[4..],out error);
        if(channelId is not null&&!channelId.Equals("hermes",StringComparison.Ordinal)&&!channelId.Equals("codex-work",StringComparison.Ordinal)&&!channelId.Equals("workbuddy",StringComparison.Ordinal)&&!channelId.Equals("minimax-code",StringComparison.Ordinal))
        {
            error="所选 AI 渠道不存在，请重新选择。";
            return null;
        }
        var selectedKind=channelId switch
        {
            "hermes"=>ConversationChannelKind.Hermes,
            "codex-work"=>ConversationChannelKind.Codex,
            "workbuddy"=>ConversationChannelKind.WorkBuddy,
            "minimax-code"=>ConversationChannelKind.MiniMaxCode,
            _=>ConversationChannelKind.Api
        };
        if(channelId is null)
        {
            selectedKind=settings.MiniMaxCodeEnabled?ConversationChannelKind.MiniMaxCode:settings.WorkBuddyEnabled?ConversationChannelKind.WorkBuddy:settings.CodexEnabled?ConversationChannelKind.Codex:settings.HermesEnabled?ConversationChannelKind.Hermes:ConversationChannelKind.Api;
        }
        if(selectedKind==ConversationChannelKind.MiniMaxCode)
        {
            try
            {
                if(!settings.MiniMaxCodeEnabled)throw new InvalidOperationException("MiniMax Code 当前未启用，请在设置中完成配置。");
                MiniMaxCodeRuntime.ValidateModel(settings.MiniMaxCodeModel);
                if(MiniMaxCodeRuntime.TryGetDesktopSession() is null)throw new InvalidOperationException("未发现 MiniMax Code 桌面版登录会话，请点击设置页的“打开 MiniMax Code”完成登录。");
                return new MiniMaxCodeAiProvider(settings.MiniMaxCodeModel);
            }
            catch(InvalidOperationException ex){error=ex.Message;return null;}
        }
        if(selectedKind==ConversationChannelKind.WorkBuddy)
        {
            try
            {
                if(!settings.WorkBuddyEnabled)throw new InvalidOperationException("WorkBuddy 当前未启用，请在设置中完成配置。");
                WorkBuddySettingsPolicy.Validate(settings.WorkBuddyModel,settings.WorkBuddyReasoningEffort);
                if(WorkBuddyAcpServer.Discover() is null)throw new InvalidOperationException("未找到本机 WorkBuddy，请安装并登录官方客户端。");
                return new WorkBuddyAiProvider(settings.WorkBuddyModel,settings.WorkBuddyReasoningEffort,settings.WorkBuddySupportsImage);
            }
            catch(InvalidOperationException ex){error=ex.Message;return null;}
        }
        if(selectedKind==ConversationChannelKind.Codex)
        {
            try
            {
                if(!settings.CodexEnabled)throw new InvalidOperationException("Codex 当前未启用，请在设置中完成配置。");
                CodexSettingsPolicy.Validate(settings);
                if(CodexAppServer.Discover() is null)throw new InvalidOperationException("未找到本机 Codex，请安装并登录官方 ChatGPT 桌面应用。");
                return new CodexAiProvider(settings.CodexModel,settings.CodexReasoningEffort,settings.CodexSupportsImage);
            }
            catch(InvalidOperationException ex){error=ex.Message;return null;}
        }
        if(selectedKind==ConversationChannelKind.Api)return aiProviderFactory.Create(settings,out error);
        if(!settings.HermesEnabled){error="Hermes 当前未启用，请在设置中完成配置。";return null;}
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
        var channels=GetConversationChannels();
        if(channels.Count>0)return true;
        error="没有可用的 AI 渠道，请先在设置中完成配置。";
        return false;
    }

    public bool IsScreenAiAvailable(out string? error)
    {
        error=null;
        if(GetConversationChannels() is {Count:>0} channels)return true;
        var provider=_aiProviderFactory.Create(Settings,out error);
        // The overlay also hosts clean text-only turns. A text model may
        // therefore expose the composer, while SendAsync still blocks visual
        // attachments when the selected model lacks image/video capability.
        return provider is not null;
    }

    public Task ReadHermesResponseAloudAsync(string text,CancellationToken cancellationToken=default)
    {
        if(!Settings.HermesEnabled||!Settings.HermesAutoReadAloud||string.IsNullOrWhiteSpace(text))return Task.CompletedTask;
        if(Volatile.Read(ref _disposed)!=0||IsExiting)return Task.CompletedTask;
        return _hermesReadAloud.SpeakAsync(_hermesRuntime,text,Settings.HermesProfile,cancellationToken);
    }

    public void StopHermesReadAloud()=>_hermesReadAloud.Stop();

    internal IReadOnlyList<ConversationHistoryEntry> GetSessionConversationHistory(string provider,string model)
    {
        lock(_sessionHistoryGate)
        {
            return _sessionConversationHistory
                .Where(entry=>string.Equals(entry.Provider,provider,StringComparison.Ordinal)&&string.Equals(entry.Model,model,StringComparison.Ordinal))
                .TakeLast(24)
                .ToArray();
        }
    }

    internal void RememberConversationHistory(ConversationHistoryEntry entry)
    {
        ArgumentNullException.ThrowIfNull(entry);
        lock(_sessionHistoryGate)
        {
            _sessionConversationHistory.Add(entry);
            const int maxEntries=100;
            if(_sessionConversationHistory.Count>maxEntries)
                _sessionConversationHistory.RemoveRange(0,_sessionConversationHistory.Count-maxEntries);
        }
    }

    internal void ClearSessionConversationHistory()
    {
        lock(_sessionHistoryGate)_sessionConversationHistory.Clear();
    }

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
        if(IsExiting||_app.Dispatcher.HasShutdownStarted){_restoreMainAfterAuxiliary=false;return;}
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
        return true;
    }
    public void Notify(string message){_tray?.ShowBalloonTip(1500,"MewuAI",LocalizationService.TranslateUiText(message),Forms.ToolTipIcon.Info);}
    internal void BeginShutdown()
    {
        if(IsExiting)return;
        CrashDiagnosticsService.MarkOperation("正在退出");IsExiting=true;_lifetime.Cancel();if(_tray is not null)_tray.Visible=false;
    }
    public void Exit() { BeginShutdown();_app.Shutdown(); }
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
        lock(_sessionHistoryGate)_sessionConversationHistory.Clear();
        DisposeSafely(_single,"SingleInstanceDispose");
        _lifetime.Dispose();
    }
    private static void DisposeSafely(IDisposable? resource,string component){try{resource?.Dispose();}catch(Exception ex){try{new PrivacyLogger().Error(component,ex);}catch{}}}

    internal sealed class LightTrayMenuRenderer : Forms.ToolStripProfessionalRenderer
    {
        private static readonly Color Background=Color.FromArgb(250,251,253);
        private static readonly Color Border=Color.FromArgb(215,222,233);
        private static readonly Color Hover=Color.FromArgb(237,242,250);

        internal LightTrayMenuRenderer():base(new LightTrayMenuColorTable()){RoundedEdges=true;}

        protected override void OnRenderItemText(Forms.ToolStripItemTextRenderEventArgs e)
        {
            if(e.Item is Forms.ToolStripMenuItem&&e.ToolStrip is Forms.ContextMenuStrip)
            {
                // DropDownMenu shares a preferred-height TextRectangle between
                // rows, even when AutoSize=false gives each row a taller height.
                // Keep its horizontal layout, but center in the actual local row
                // on every paint so DPI/font changes cannot leave a stale offset.
                var textBounds=e.TextRectangle;
                e.TextRectangle=new Rectangle(textBounds.X,0,textBounds.Width,e.Item.Height);
                e.TextFormat=(e.TextFormat&~Forms.TextFormatFlags.Bottom)
                    |Forms.TextFormatFlags.VerticalCenter|Forms.TextFormatFlags.SingleLine;
            }
            base.OnRenderItemText(e);
        }

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
