// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using ComboBox = System.Windows.Controls.ComboBox;
using TabControl = System.Windows.Controls.TabControl;
using TextBox = System.Windows.Controls.TextBox;

/// <summary>
/// Fresh-process saved-language contract using real, credential-free settings
/// in a marked temporary directory. Windows are constructed but never shown;
/// only the window's direct Loaded event is raised, never page Loaded probes.
/// This does not reproduce or explain a previously running app's language state.
/// </summary>
internal static class LocalizationStartupReplay
{
    private const BindingFlags Private = BindingFlags.Instance | BindingFlags.NonPublic;
    private const string UserProviderName = "保存";
    private const string UserModel = "设置";
    private const string UserSender = "删除";
    private const string UserKnowledgeBase = "常规";
    private static readonly JsonSerializerOptions Json = new() { WriteIndented = true };
    private sealed record Scenario(string Name, string SystemCulture, string Preference, string ExpectedCulture,
        string? NextPreference = null, bool ExistingSettings = false, string? SharedSettings = null);
    private sealed record Result(string Scenario, bool Passed, int ProcessId, string ProcessStartUtc,
        string ProductSha256, string HarnessSha256, string[] Checks, string[] Failures,
        bool WindowShown = false, bool HostStarted = false, bool ProductionSettingsUsed = false,
        bool ProviderOrAccountProbeInvoked = false, bool ClipboardUsed = false);

    internal static void Run(string[] args)
    {
        var childName = args.FirstOrDefault(arg => arg.StartsWith("--language-case=", StringComparison.Ordinal));
        var cwd = childName is null ? ReplayOutputDirectory.PrepareWorkingDirectory("localization-startup")
            : ReplayOutputDirectory.ValidateDirectory(Environment.CurrentDirectory);
        if (childName is not null)
        {
            var scenario = JsonSerializer.Deserialize<Scenario>(File.ReadAllText(Path.Combine(cwd, "scenario.json")))
                ?? throw new InvalidOperationException("Missing synthetic language scenario.");
            if (scenario.Name != childName["--language-case=".Length..]) throw new InvalidOperationException("Scenario identity mismatch.");
            RunChild(cwd, scenario);
            return;
        }
        RunMatrix(cwd);
    }

    private static void RunMatrix(string cwd)
    {
        var output = Path.Combine(cwd, ".codex-build", "localization-startup");
        Directory.CreateDirectory(output);
        var scenarios = new Scenario[]
        {
            new("system-zh-saved-en", "zh-CN", "en-US", "en-US"),
            new("system-en-saved-zh", "en-US", "zh-CN", "zh-CN"),
            new("system-zh-fallback", "zh-CN", "system", "zh-CN"),
            new("system-en-fallback", "en-US", "system", "en-US"),
            new("system-fr-fallback", "fr-FR", "system", "en-US"),
            new("restart-en-save-zh", "zh-CN", "en-US", "en-US", "zh-CN", SharedSettings: "restart-settings.json"),
            new("restart-zh-save-en", "zh-CN", "zh-CN", "zh-CN", "en-US", true, "restart-settings.json"),
            new("restart-en-final", "zh-CN", "en-US", "en-US", ExistingSettings: true, SharedSettings: "restart-settings.json")
        };
        var results = new List<Result>();
        var failures = new List<string>();
        foreach (var scenario in scenarios)
        {
            var directory = Path.Combine(output, scenario.Name);
            Directory.CreateDirectory(directory);
            File.WriteAllText(Path.Combine(directory, "scenario.json"), JsonSerializer.Serialize(scenario, Json), new UTF8Encoding(false));
            var start = new ProcessStartInfo(Environment.ProcessPath ?? throw new InvalidOperationException("No harness executable."))
            {
                WorkingDirectory = directory, UseShellExecute = false, CreateNoWindow = true, WindowStyle = ProcessWindowStyle.Hidden
            };
            start.ArgumentList.Add("--verify-localization-startup");
            start.ArgumentList.Add("--language-case=" + scenario.Name);
            start.Environment["TEMP"] = directory;
            start.Environment["TMP"] = directory;
            using var process = Process.Start(start) ?? throw new InvalidOperationException("Could not launch fresh-process language replay.");
            if (!process.WaitForExit(45_000))
            {
                // This is the exact child created above, never a production PID.
                process.Kill();
                process.WaitForExit();
                failures.Add(scenario.Name + ": isolated child timed out");
                break;
            }
            var resultPath = Path.Combine(directory, "result.json");
            if (!File.Exists(resultPath)) { failures.Add(scenario.Name + ": isolated child produced no report"); continue; }
            var result = JsonSerializer.Deserialize<Result>(File.ReadAllText(resultPath))
                ?? throw new InvalidOperationException("Invalid child report.");
            results.Add(result);
            if (process.ExitCode != 0 || !result.Passed) failures.Add(scenario.Name + ": language contract failed");
        }
        if (results.Count != scenarios.Length) failures.Add("Not every fresh-process scenario completed.");
        if (results.Select(result => (result.ProcessId, result.ProcessStartUtc)).Distinct().Count() != results.Count)
            failures.Add("Language scenarios did not use distinct processes.");
        File.WriteAllText(Path.Combine(output, "result.json"), JsonSerializer.Serialize(new
        {
            passed = failures.Count == 0, freshProcesses = results.Count,
            checks = results.Sum(result => result.Checks.Length), failures, scenarios = results,
            scope = "Synthetic saved-language startup and unshown control-tree regression; no production startup or desktop input."
        }, Json), new UTF8Encoding(false));
        Environment.ExitCode = failures.Count == 0 ? 0 : 1;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void RunChild(string directory, Scenario scenario)
    {
        var checks = new List<string>();
        var failures = new List<string>();
        App? application = null;
        AppHost? host = null;
        SettingsWindow? settingsWindow = null;
        MainWindow? main = null;
        try
        {
            CultureInfo.CurrentCulture = CultureInfo.GetCultureInfo(scenario.SystemCulture);
            CultureInfo.CurrentUICulture = CultureInfo.GetCultureInfo(scenario.SystemCulture);
            PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
            var credentialDirectory = Path.Combine(directory, "empty-credentials");
            var credentials = new CredentialService(credentialDirectory);
            var headers = new ProviderHeaderCredentialService(credentials);
            var path = scenario.SharedSettings is null ? Path.Combine(directory, "settings.json")
                : Path.Combine(Directory.GetParent(directory)!.FullName, scenario.SharedSettings);
            ReplayOutputDirectory.ValidateDirectory(Path.GetDirectoryName(path)!);
            var settingsErrors = new List<string>();
            var service = new SettingsService(path, headers, (component, error) => settingsErrors.Add(component + ":" + error.GetType().Name));
            if (!scenario.ExistingSettings) service.Save(CreateSettings(scenario.Preference, directory));
            var saved = service.Load();
            Require(saved.UiLanguage == scenario.Preference, "real-settings-service-loads-saved-language", checks);
            Require(saved.ConfigurationErrors.Count == 0 && settingsErrors.Count == 0, "synthetic-settings-valid-without-fallback", checks);
            Require(Directory.GetFiles(credentialDirectory).Length == 0, "no-synthetic-credential-files-created", checks);
            application = new App();
            application.InitializeComponent();
            // App.Run would invoke production OnStartup. This replay never calls it.
            host = new AppHost(application, null, "MewuAI.LocalizationReplay." + Guid.NewGuid().ToString("N"));
            typeof(AppHost).GetProperty(nameof(AppHost.Settings))!.SetValue(host, saved);
            Set(host, "_settingsService", service);
            Set(host, "_aiProviderFactory", new AiProviderFactory(credentials, null));
            Field<IDisposable>(host, "_hermesRuntime").Dispose();
            Set(host, "_hermesRuntime", new HermesRuntimeService(new HermesBackendService(
                new HermesDiscoveryService((_, _) => null, _ => string.Empty, new MissingFileSystem()))));
            InitializeLikeProduction(saved);
            Require(LocalizationService.CultureName == scenario.ExpectedCulture, "saved-preference-wins-over-system-culture", checks);
            var english = scenario.ExpectedCulture == "en-US";
            Require(LocalizationService.T("中文动态状态", "English dynamic status") == (english ? "English dynamic status" : "中文动态状态"),
                "dynamic-localization-after-first-initialize", checks);
            main = new MainWindow(host);
            settingsWindow = new SettingsWindow(host, headers);
            var settingsBeforeInspection = JsonSerializer.Serialize(saved);
            VerifyWindows(main, settingsWindow, saved, english, checks);
            if (scenario.Name == "system-zh-saved-en") RenderEnglishEvidence(settingsWindow, directory, checks);
            Require(JsonSerializer.Serialize(saved) == settingsBeforeInspection, "page-inspection-does-not-mutate-saved-settings", checks);
            VerifyPreviewResolutionRoundTrip(settingsWindow, saved, directory, headers, checks);
            Require(JsonSerializer.Serialize(saved) == settingsBeforeInspection, "preview-choice-replay-does-not-mutate-host-settings", checks);
            Require(LocalizationService.CultureName == scenario.ExpectedCulture, "resources-main-and-settings-do-not-reset-language", checks);
            if (scenario.NextPreference is not null)
            {
                var language = Field<ComboBox>(settingsWindow, "_uiLanguage");
                language.SelectedItem = language.Items.OfType<ComboBoxItem>().Single(item => (string)item.Tag == scenario.NextPreference);
                saved.UiLanguage = (string)((ComboBoxItem)language.SelectedItem).Tag;
                service.Save(saved);
                Require(service.Load().UiLanguage == scenario.NextPreference, "actual-language-choice-persists-through-real-save-load", checks);
                Require(LocalizationService.CultureName == scenario.ExpectedCulture, "saving-preference-does-not-mutate-current-process-language", checks);
            }
            Require(!settingsWindow.IsVisible && !main.IsVisible && new WindowInteropHelper(settingsWindow).Handle == IntPtr.Zero
                && new WindowInteropHelper(main).Handle == IntPtr.Zero, "real-windows-remain-unshown-without-native-handles", checks);
            Require(settingsErrors.Count == 0 && Directory.GetFiles(credentialDirectory).Length == 0, "no-provider-credentials-or-settings-errors", checks);
        }
        catch (Exception error)
        {
            while (error is TargetInvocationException { InnerException: { } inner }) error = inner;
            failures.Add(error is ReplayFailure ? error.Message : error.GetType().FullName ?? error.GetType().Name);
            if (error is not ReplayFailure) failures.AddRange((error.StackTrace ?? "").Split('\n').Select(line => line.Split(" in ")[0].Trim()));
        }
        finally
        {
            // Do not pump Application startup while closing these unshown windows.
            try { settingsWindow?.Close(); } catch (Exception error) { failures.Add("settings-close:" + error.GetType().Name); }
            try { main?.Close(); } catch (Exception error) { failures.Add("main-close:" + error.GetType().Name); }
            try { host?.Dispose(); } catch (Exception error) { failures.Add("isolated-host-dispose:" + error.GetType().Name); }
            try { application?.Shutdown(); } catch (Exception error) { failures.Add("application-shutdown:" + error.GetType().Name); }
            using var process = Process.GetCurrentProcess();
            var result = new Result(scenario.Name, failures.Count == 0, process.Id, process.StartTime.ToUniversalTime().ToString("O"),
                Hash(typeof(AppHost).Assembly.Location), Hash(typeof(LocalizationStartupReplay).Assembly.Location), checks.ToArray(), failures.ToArray());
            File.WriteAllText(Path.Combine(directory, "result.json"), JsonSerializer.Serialize(result, Json), new UTF8Encoding(false));
            Environment.ExitCode = failures.Count == 0 ? 0 : 1;
        }
    }

    // Kept separate from any typeof(LocalizationService)/reflection reads so
    // every child really calls Initialize first, as production startup does.
    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void InitializeLikeProduction(AppSettings settings)
        => LocalizationService.Initialize(settings.UiLanguage, CultureInfo.CurrentUICulture);

    private static AppSettings CreateSettings(string language, string directory) => new()
    {
        UiLanguage = language, DefaultProviderId = "localization-synthetic", SaveConversationHistory = false,
        ThinkingGlowEnabled = false, HermesModel = "synthetic-language-model",
        Providers = [new() { Id = "localization-synthetic", Name = UserProviderName, Type = "OpenAICompatible",
            BaseUrl = "https://example.invalid/v1", Model = UserModel, AuthMode = "bearer", CredentialId = "" }],
        NetEaseMailFromName = UserSender, ObsidianAttachFolder = "保存", ObsidianNoteFolder = "设置",
        ObsidianVaultPath = Path.Combine(directory, "synthetic-vault"),
        ImaKnowledgeBaseId = "synthetic-knowledge-base", ImaKnowledgeBaseName = UserKnowledgeBase
    };

    private static void VerifyPreviewResolutionRoundTrip(SettingsWindow window, AppSettings saved, string directory,
        ProviderHeaderCredentialService headers, List<string> checks)
    {
        var choice = Field<ComboBox>(window, "_videoPreviewResolution");
        var originalChoice = choice.SelectedItem;
        var service = new SettingsService(Path.Combine(directory, "preview-resolution-roundtrip.json"), headers, null);
        // Exercise the real selection reader and persistence with a separate synthetic
        // snapshot. Full SettingsWindow.Save also manages accounts/startup and is not invoked.
        var readChoice = typeof(SettingsWindow).GetMethod("ReadNumericChoice", BindingFlags.Static | BindingFlags.NonPublic)!;
        try
        {
            foreach (var percent in new[] { 100, 75, 50 })
            {
                choice.SelectedItem = choice.Items.OfType<ComboBoxItem>().Single(item => (int)item.Tag == percent);
                var candidate = JsonSerializer.Deserialize<AppSettings>(JsonSerializer.Serialize(saved))!;
                candidate.VideoPreviewResolutionPercent = (int)readChoice.Invoke(null, new object[] { choice, 100 })!;
                service.Save(candidate);
                var loaded = service.Load();
                Require(loaded.VideoPreviewResolutionPercent == percent && loaded.ConfigurationErrors.Count == 0,
                    $"actual-preview-choice-{percent}-persists-through-real-save-load", checks);
                Require(loaded.RecordingFps == saved.RecordingFps && loaded.RecordingQuality == saved.RecordingQuality
                    && loaded.GifFps == saved.GifFps && loaded.RecordSystemAudio == saved.RecordSystemAudio
                    && loaded.RecordMicrophone == saved.RecordMicrophone,
                    $"preview-choice-{percent}-preserves-recording-settings", checks);
            }
        }
        finally { choice.SelectedItem = originalChoice; }
    }

    private static void VerifyWindows(MainWindow main, SettingsWindow settings, AppSettings saved, bool english, List<string> checks)
    {
        Require(FrameworkElement.LoadedEvent.RoutingStrategy == RoutingStrategy.Direct, "loaded-event-is-direct-and-does-not-broadcast-to-pages", checks);
        var pageLoads = 0;
        var externalPages = new[] { "_codexSettings", "_workBuddySettings", "_miniMaxCodeSettings", "_qqMailSettings",
            "_netEaseMailSettings", "_dingTalkSettings", "_feishuSettings", "_obsidianSettings", "_imaSettings" };
        foreach (var field in externalPages) Field<FrameworkElement>(settings, field).Loaded += (_, _) => pageLoads++;
        main.RaiseEvent(new RoutedEventArgs(FrameworkElement.LoadedEvent, main));
        settings.RaiseEvent(new RoutedEventArgs(FrameworkElement.LoadedEvent, settings));
        Require(((TextBlock)main.FindName("AiStatusTitle")).Text == Choose(english, "暂未设置AI功能", "AI features are not set up"),
            "real-main-dynamic-ai-status", checks);
        Require(((TextBlock)main.FindName("CaptureSubtitle")).Text == Choose(english, "截图、OCR、标注和录屏", "Capture, OCR, annotate, and record"),
            "real-main-dynamic-capture-subtitle", checks);
        Require(settings.Title == Choose(english, "喵呜AI 设置", "MewuAI Settings"), "real-settings-window-title", checks);
        var topTabs = Descendants(settings).OfType<TabControl>().Single(tab => tab.Items.Count == 8);
        var chinese = new[] { "常规", "捕获", "录屏", "AI", "MCP", "语音", "隐私", "关于" };
        var translated = new[] { "General", "Capture", "Recording", "AI", "MCP", "Voice", "Privacy", "About" };
        for (var index = 0; index < topTabs.Items.Count; index++)
        {
            var tab = (TabItem)topTabs.Items[index];
            Require((string)tab.Header == (english ? translated[index] : chinese[index]), "top-tab-" + translated[index], checks);
        }
        var language = Field<ComboBox>(settings, "_uiLanguage");
        Require(AutomationProperties.GetName(language) == Choose(english, "界面语言", "Display language"), "general-language-accessible-name", checks);
        Require((string)((ComboBoxItem)language.SelectedItem).Tag == saved.UiLanguage, "general-language-selection-matches-loaded-preference", checks);
        Require(HasText((DependencyObject)((TabItem)topTabs.Items[0]).Content, Choose(english, "语言设置将在重新启动喵呜AI后生效。", "Language changes take effect after restarting MewuAI.")),
            "general-restart-language-explanation", checks);
        var previewResolution = Field<ComboBox>(settings, "_videoPreviewResolution");
        var previewChoices = previewResolution.Items.OfType<ComboBoxItem>().ToArray();
        Require(previewChoices.Select(item => (int)item.Tag).SequenceEqual(new[] { 100, 75, 50 }), "preview-resolution-has-only-supported-scales", checks);
        Require(previewChoices.Select(item => (string)item.Content).SequenceEqual(new[] { Choose(english, "原始（100%）", "Original (100%)"), "75%", "50%" }),
            "preview-resolution-choice-labels-localized", checks);
        Require(AutomationProperties.GetName(previewResolution) == Choose(english, "预览分辨率", "Preview resolution"), "preview-resolution-accessible-name", checks);
        Require((int)previewResolution.SelectedValue == saved.VideoPreviewResolutionPercent && saved.VideoPreviewResolutionPercent == 100,
            "new-preview-settings-default-to-original-resolution", checks);
        Require(HasText((DependencyObject)((TabItem)topTabs.Items[2]).Content, Choose(english,
            "新打开的视频预览生效；保存与发送的分辨率不变。",
            "Applies to newly opened video previews; saved and sent video resolution stays unchanged.")), "preview-only-scope-explained", checks);
        var backends = Field<AiSettingsTabs>(settings, "_backendSelector");
        var backendNames = new[] { "API", "Hermes", "Codex", "WorkBuddy", "MiniMax Code" };
        Require(backends.Tabs.Items.Count == backendNames.Length, "all-five-ai-backends-present", checks);
        for (var index = 0; index < backendNames.Length; index++)
        {
            backends.Tabs.SelectedIndex = index;
            settings.RaiseEvent(new RoutedEventArgs(FrameworkElement.LoadedEvent, settings));
            var tab = (TabItem)backends.Tabs.Items[index];
            Require((string)tab.Header == backendNames[index], "ai-tab-" + backendNames[index], checks);
            var content = (DependencyObject)tab.Content;
            var representative = index switch
            {
                0 => Choose(english, "我的 API 连接", "My API connections"),
                1 => Choose(english, "Agent / 人格", "Agent / Profile"),
                _ => Choose(english, "模型", "Model")
            };
            Require(HasText(content, representative), "ai-page-label-" + backendNames[index], checks);
        }
        VerifyPage(settings, "_miniMaxCodeSettings", english, checks,
            ("刷新状态", "Refresh status"), ("打开 MiniMax Code", "Open MiniMax Code"), ("测试连接", "Test connection"));
        VerifyPage(settings, "_codexSettings", english, checks, ("模型", "Model"), ("思考程度", "Reasoning effort"));
        VerifyPage(settings, "_workBuddySettings", english, checks, ("模型", "Model"), ("思考程度", "Reasoning effort"));
        VerifyPage(settings, "_qqMailSettings", english, checks, ("QQ 邮箱", "QQ Mail"), ("扫码授权", "Scan QR code to authorize"), ("清除授权", "Clear authorization"));
        VerifyPage(settings, "_netEaseMailSettings", english, checks, ("网易邮箱", "NetEase Mail"), ("保存授权码", "Save auth code"), ("清除授权码", "Clear auth code"));
        VerifyPage(settings, "_dingTalkSettings", english, checks, ("钉钉", "DingTalk"), ("保存 Secret", "Save secret"), ("清除 Secret", "Clear secret"));
        VerifyPage(settings, "_feishuSettings", english, checks, ("飞书", "Feishu"), ("保存 Secret", "Save secret"), ("清除 Secret", "Clear secret"));
        VerifyPage(settings, "_obsidianSettings", english, checks, ("Obsidian", "Obsidian"), ("刷新 vault 列表", "Refresh vault list"));
        VerifyPage(settings, "_imaSettings", english, checks, ("ima", "ima"), ("保存 API Key", "Save API Key"), ("清除 API Key", "Clear API Key"));
        foreach (var field in new[] { "_qqMailSettings", "_netEaseMailSettings", "_dingTalkSettings", "_feishuSettings" })
            Require(Field<TextBlock>(Field<object>(settings, field), "_status").Text == Choose(english, "正在读取本机授权状态…", "Reading local authorization status…"),
                field + "-initial-status-without-account-probe", checks);
        VerifyUserData(settings, checks);
        VerifyOwnedMessages(english, settings, checks);
        if (english)
        {
            foreach (var page in externalPages) RequireEnglishSurface(Field<DependencyObject>(settings, page), page, checks);
            RequireEnglishSurface((DependencyObject)((TabItem)topTabs.Items[0]).Content, "General", checks);
            var miniModel = Field<ComboBox>(Field<object>(settings, "_miniMaxCodeSettings"), "_model");
            Require(miniModel.Items.Cast<object>().All(item => !HasHan(item.ToString())), "minimax-model-capability-labels-english", checks);
        }
        Require(pageLoads == 0, "no-page-loaded-account-or-provider-probes", checks);
        foreach (var field in externalPages)
            Require(!Field<FrameworkElement>(settings, field).IsLoaded, field + "-not-physically-loaded", checks);
    }

    private static void VerifyUserData(SettingsWindow settings, List<string> checks)
    {
        var api = Field<DependencyObject>(settings, "_apiConnections");
        Require(Descendants(api).OfType<TextBlock>().Any(text => text.Text == UserProviderName), "user-provider-display-name-not-translated", checks);
        Require(Field<ComboBox>(settings, "_model").Text == UserModel, "user-model-id-not-translated", checks);
        Require(Field<TextBox>(Field<object>(settings, "_netEaseMailSettings"), "_fromName").Text == UserSender, "user-mail-sender-not-translated", checks);
        var obsidian = Field<object>(settings, "_obsidianSettings");
        Require(Field<TextBox>(obsidian, "_attach").Text == "保存" && Field<TextBox>(obsidian, "_notes").Text == "设置", "user-note-paths-not-translated", checks);
        var knowledge = Field<ComboBox>(Field<object>(settings, "_imaSettings"), "_knowledgeBase");
        Require(knowledge.SelectedItem?.ToString() == UserKnowledgeBase, "user-knowledge-base-name-not-translated", checks);
    }

    private static void VerifyOwnedMessages(bool english, SettingsWindow settings, List<string> checks)
    {
        const string external = "  第三方原文：保存/设置\r\n preserve whitespace  ";
        Require(MiniMaxCodeSettingsPage.FormatErrorMessage(external) == external
            && CodexSettingsPage.FormatErrorMessage(external) == external
            && WorkBuddySettingsPage.FormatErrorMessage(external) == external, "third-party-error-details-remain-byte-for-byte-unchanged", checks);
        Require(MiniMaxCodeSettingsPage.FormatErrorMessage("MiniMax Code 未返回有效正文。")
            == Choose(english, "MiniMax Code 未返回有效正文。", "MiniMax Code did not return a valid response."), "minimax-owned-error-is-localized", checks);
        Require(CodexSettingsPage.FormatErrorMessage("响应缺少必要字段。")
            == Choose(english, "响应缺少必要字段。", "The response is missing required fields."), "codex-owned-error-is-localized", checks);
        Require(WorkBuddySettingsPage.FormatErrorMessage("WorkBuddy HTTP 接口返回 401。")
            == Choose(english, "WorkBuddy HTTP 接口返回 401。", "The WorkBuddy HTTP endpoint returned 401."), "workbuddy-owned-error-preserves-http-code", checks);
        const string waiting = "WorkBuddy 本机接口在 20 秒内未就绪，请确认官方客户端已登录后重试。";
        var waitingMessage = WorkBuddySettingsPage.FormatErrorMessage(waiting + external);
        Require(waitingMessage.EndsWith(external, StringComparison.Ordinal), "workbuddy-owned-prefix-preserves-external-detail-whitespace", checks);
        var custom = new MiniMaxCodeModel("minimax/MiniMax-M3", "保存");
        Require(MiniMaxCodeSettingsPage.FormatModelName(custom) == custom.Name, "custom-model-name-is-not-translated", checks);
        var page = Field<MiniMaxCodeSettingsPage>(settings, "_miniMaxCodeSettings");
        var model = Field<ComboBox>(page, "_model");
        for (var index = 0; index < model.Items.Count; index++)
        {
            model.SelectedIndex = index;
            Require(ReferenceEquals(page.SelectedModel, MiniMaxCodeRuntime.KnownModels[index]), "minimax-choice-retains-original-model-object-" + index, checks);
            Require(page.SelectedModel!.Model == MiniMaxCodeRuntime.KnownModels[index].Model, "minimax-choice-retains-persistent-model-id-" + index, checks);
            Require(english ? !HasHan(model.SelectedItem.ToString()) : model.SelectedItem.ToString() == page.SelectedModel.Name,
                "minimax-model-display-language-" + index, checks);
        }
    }

    private static void RenderEnglishEvidence(SettingsWindow settings, string directory, List<string> checks)
    {
        // No Application.Run/dispatcher frame: these are actual WPF visuals
        // laid out and rendered offscreen, not a visible desktop replay.
        var topTabs = Descendants(settings).OfType<TabControl>().Single(tab => tab.Items.Count == 8);
        topTabs.SelectedIndex = 3;
        Field<AiSettingsTabs>(settings, "_backendSelector").Tabs.SelectedIndex = AiSettingsTabs.MiniMaxCodeIndex;
        var page = Field<FrameworkElement>(settings, "_miniMaxCodeSettings");
        var root = (FrameworkElement)settings.Content;
        foreach (var width in new[] { 600, 760 })
        {
            var size = new System.Windows.Size(width, 574);
            root.Measure(size);
            root.Arrange(new Rect(new System.Windows.Point(), size));
            root.UpdateLayout();
            Require(page.ActualWidth > 200, "offscreen-minimax-page-realized-" + width, checks);
            foreach (var button in Descendants(page).OfType<System.Windows.Controls.Button>())
            {
                var bounds = button.TransformToAncestor(page).TransformBounds(new Rect(new System.Windows.Point(), button.RenderSize));
                Require(button.ActualWidth > 0 && button.ActualHeight >= 30 && bounds.Left >= -.5 && bounds.Right <= page.ActualWidth + .5,
                    "offscreen-minimax-action-fits-" + width + "-" + button.Content, checks);
            }
            var bitmap = new RenderTargetBitmap(width, 574, 96, 96, PixelFormats.Pbgra32);
            bitmap.Render(root);
            var encoder = new PngBitmapEncoder();
            encoder.Frames.Add(BitmapFrame.Create(bitmap));
            using var stream = File.Create(Path.Combine(directory, "english-minimax-" + width + ".png"));
            encoder.Save(stream);
        }
        Require(!page.IsLoaded && new WindowInteropHelper(settings).Handle == IntPtr.Zero,
            "offscreen-render-keeps-page-unloaded-and-no-window-handle", checks);
    }

    private static void VerifyPage(SettingsWindow window, string field, bool english, List<string> checks, params (string Chinese, string English)[] labels)
    {
        var page = Field<DependencyObject>(window, field);
        foreach (var label in labels) Require(HasText(page, Choose(english, label.Chinese, label.English)), field + "-label-" + label.English, checks);
        foreach (var button in Descendants(page).OfType<System.Windows.Controls.Button>())
            if (button.Content is string content && !string.IsNullOrEmpty(AutomationProperties.GetName(button)))
                Require(AutomationProperties.GetName(button) == content, field + "-button-accessible-name-" + content, checks);
    }

    private static void RequireEnglishSurface(DependencyObject page, string name, List<string> checks)
    {
        foreach (var element in Descendants(page).OfType<FrameworkElement>())
        {
            if (Excluded(element)) continue;
            foreach (var value in StaticUiText(element))
                Require(!HasHan(value), name + "-english-" + element.GetType().Name + "-" + value, checks);
        }
    }

    private static IEnumerable<string> StaticUiText(FrameworkElement element)
    {
        if (element is TextBlock text) yield return text.Text;
        if (element is ContentControl { Content: string content }) yield return content;
        if (element is HeaderedContentControl { Header: string header }) yield return header;
        if (element.ToolTip is string tooltip) yield return tooltip;
        yield return AutomationProperties.GetName(element);
    }

    private static bool Excluded(DependencyObject element)
    {
        for (DependencyObject? current = element; current is not null; current = current is FrameworkElement framework ? framework.Parent : LogicalTreeHelper.GetParent(current))
            if (LocalizationService.GetExcludeFromLocalization(current)) return true;
        return false;
    }

    private static IEnumerable<DependencyObject> Descendants(DependencyObject root)
    {
        var pending = new Stack<DependencyObject>();
        var seen = new HashSet<DependencyObject>();
        pending.Push(root);
        while (pending.Count > 0)
        {
            var current = pending.Pop();
            if (!seen.Add(current)) continue;
            yield return current;
            foreach (var child in LogicalTreeHelper.GetChildren(current).OfType<DependencyObject>()) pending.Push(child);
            if (current is not Visual and not System.Windows.Media.Media3D.Visual3D) continue;
            for (var index = 0; index < VisualTreeHelper.GetChildrenCount(current); index++) pending.Push(VisualTreeHelper.GetChild(current, index));
        }
    }

    private static bool HasText(DependencyObject root, string expected) => Descendants(root).OfType<FrameworkElement>()
        .Any(element => StaticUiText(element).Contains(expected, StringComparer.Ordinal));
    private static bool HasHan(string? value) => value?.Any(character => character is >= '\u3400' and <= '\u9fff') == true;
    private static string Choose(bool english, string chinese, string translated) => english ? translated : chinese;
    private static string Hash(string path) => Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path)));
    private static T Field<T>(object value, string name) => (T)(value.GetType().GetField(name, Private)?.GetValue(value)
        ?? throw new ReplayFailure("Missing replay field: " + name));
    private static void Set(object value, string name, object fieldValue) => value.GetType().GetField(name, Private)!.SetValue(value, fieldValue);
    private static void Require(bool condition, string name, List<string> checks)
    {
        if (!condition) throw new ReplayFailure(name);
        checks.Add(name);
    }
    private sealed class ReplayFailure(string message) : Exception(message);
    private sealed class MissingFileSystem : IHermesDiscoveryFileSystem
    {
        public bool TryGetAttributes(string path, out FileAttributes attributes) { attributes = default; return false; }
        public bool TryGetDriveType(string rootPath, out DriveType driveType) { driveType = default; return false; }
    }

}
