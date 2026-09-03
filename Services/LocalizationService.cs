using System.ComponentModel;
using System.Globalization;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Markup;

namespace mewu_ai_Assistant.Services;

internal enum AppLanguage { English,SimplifiedChinese }

internal static class LocalizationService
{
    private static readonly Dictionary<string,string> English=new(StringComparer.Ordinal)
    {
        ["喵呜AI"]="MewuAI",["喵呜AI 屏幕助手"]="MewuAI Screen Assistant",["喵呜AI 设置"]="MewuAI Settings",["喵呜AI 贴图"]="MewuAI Pinned Image",["喵呜AI 贴视频"]="MewuAI Pinned Video",
        ["设置"]="Settings",["打开主界面"]="Open MewuAI",["退出"]="Quit",["最小化"]="Minimize",["隐藏到托盘"]="Hide to tray",["关闭设置"]="Close settings",["关闭设置窗口"]="Close the Settings window",
        ["屏幕助手"]="Screen Assistant",["圈选并直接分析"]="Select an area and analyze it",["截图、OCR、标注和录屏"]="Capture, OCR, annotate, and record",["截图、OCR、标注和录屏可用"]="Capture, OCR, annotation, and recording are available",
        ["暂未设置AI功能"]="AI features are not set up",["智能体已接入"]="Agent connected",["AI模型已接入"]="AI model connected",["未选择模型"]="No model selected",["未配置 AI 模型"]="No AI model configured",
        ["引用当前区域到对话"]="Reference this region in the conversation",["引用当前区域"]="Reference current region",["添加区域；Shift+拖动可新增重叠区域"]="Add a region; Shift-drag to add an overlapping region",["添加截图区域"]="Add capture region",["删除当前区域 (Delete)"]="Delete current region (Delete)",["删除当前区域"]="Delete current region",
        ["原位翻译 (T)"]="Translate in place (T)",["原位翻译"]="Translate in place",["原位文字识别 (O)"]="Select text with OCR (O)",["原位文字识别"]="Select text with OCR",["AI 表格识别"]="Recognize table with AI",["识别当前区域中的表格"]="Recognize tables in the current region",
        ["贴图 (P)"]="Pin image (P)",["原位贴图或贴视频"]="Pin image or video",["原位标注 (D)"]="Annotate in place (D)",["原位标注"]="Annotate in place",["播放 / 暂停视频"]="Play / pause video",["播放或暂停视频"]="Play or pause video",
        ["复制 (C)"]="Copy (C)",["复制当前区域"]="Copy current region",["保存 (S)"]="Save (S)",["保存当前区域"]="Save current region",["区域录屏 (R)"]="Record region (R)",["录制当前区域"]="Record current region",["滚动长截图"]="Scrolling capture",
        ["选择、移动或删除标注"]="Select, move, or delete annotations",["选择标注工具"]="Select annotation tool",["画笔"]="Pen",["画笔工具"]="Pen tool",["高亮"]="Highlighter",["高亮标注工具"]="Highlighter tool",["矩形"]="Rectangle",["矩形标注工具"]="Rectangle tool",
        ["椭圆；按 Shift 绘制正圆"]="Ellipse; hold Shift for a circle",["椭圆标注工具"]="Ellipse tool",["箭头"]="Arrow",["箭头标注工具"]="Arrow tool",["矩形马赛克"]="Rectangular pixelation",["矩形马赛克工具"]="Pixelation tool",["文本框"]="Text box",["文本框标注工具"]="Text annotation tool",
        ["自动递增实心序号"]="Auto-numbered marker",["序号标注工具"]="Number marker tool",["橡皮；每次只擦除最上层对象"]="Eraser; removes one topmost object at a time",["橡皮擦工具"]="Eraser tool",["红色"]="Red",["使用红色标注"]="Use red annotations",["蓝色"]="Blue",["使用蓝色标注"]="Use blue annotations",
        ["选择任意 RGB 颜色"]="Choose any RGB color",["RGB颜色选择器"]="RGB color picker",["切换文字荧光底色"]="Toggle text highlight",["文字荧光底色"]="Text highlight",["电脑已安装字体"]="Installed fonts",["标注字体"]="Annotation font",["字号"]="Font size",["标注字号"]="Annotation font size",
        ["撤销"]="Undo",["撤销标注"]="Undo annotation",["重做"]="Redo",["重做标注"]="Redo annotation",["清空标注"]="Clear annotations",["清空全部标注"]="Clear all annotations",["完成标注"]="Finish annotating",
        ["滚轮截长图"]="Scrolling capture",["已采集 1 段"]="1 segment captured",["完成长截图"]="Finish scrolling capture",["取消长截图"]="Cancel scrolling capture",["完成"]="Finish",["取消"]="Cancel",["录制中"]="Recording",["暂停"]="Pause",["暂停或继续录屏"]="Pause or resume recording",["停止并原位预览"]="Stop and preview in place",["停止录屏并原位预览"]="Stop recording and preview in place",
        ["查看提问和历史对话"]="View questions and conversation history",["查看提问与历史"]="View questions and history",["AI 回答"]="AI response",["同时复制为 Excel 表格、Markdown 和图片"]="Copy as an Excel table, Markdown, and an image",["复制识别出的表格"]="Copy recognized table",["复制表格"]="Copy table",["正在思考…"]="Thinking…",["Hermes 运行"]="Hermes activity",
        ["输入文字问题，或先圈选/上传要分析的内容…"]="Ask a question, or select/upload something to analyze…",["继续输入关于引用区域的问题…"]="Ask a follow-up about the referenced regions…",["引用截图或视频"]="Reference an image or video",["上传图片或文件"]="Upload an image or file",["语音输入"]="Voice input",["发送"]="Send",["发送问题"]="Send question",["拖动可连续框选多个区域 · Enter 发送 · Shift+Enter 换行"]="Drag to select multiple regions · Enter to send · Shift+Enter for a new line",
        ["常规"]="General",["捕获"]="Capture",["录屏"]="Recording",["语音"]="Voice",["隐私"]="Privacy",["关于"]="About",["保存"]="Save",["界面语言"]="Display language",["语言设置将在重新启动喵呜AI后生效。"]="Language changes take effect after restarting MewuAI.",["启动与快捷键"]="Startup and shortcuts",["登录 Windows 后自动启动"]="Start automatically when I sign in to Windows",["全局截图快捷键"]="Global capture shortcut",
        ["点击上面的输入框，然后直接按下新的组合键（至少包含 Shift、Alt 或 Ctrl）。"]="Click the field above, then press a new shortcut (include Shift, Alt, or Ctrl).",["恢复默认 Shift + Alt + S"]="Restore Shift + Alt + S",["关闭主窗口不会退出；请使用托盘菜单退出。"]="Closing the main window keeps MewuAI running. Use the tray menu to quit.",
        ["延时截图"]="Capture delay",["默认图片格式"]="Default image format",["选区外暗化程度"]="Dim area outside selection",["截图包含系统鼠标指针"]="Include the system pointer in captures",["截图、OCR、复制和保存均在本地完成。"]="Capture, OCR, copying, and saving are all performed locally.",
        ["MP4 帧率"]="MP4 frame rate",["MP4 质量"]="MP4 quality",["GIF 帧率"]="GIF frame rate",["录屏包含系统鼠标指针"]="Include the system pointer in recordings",["自动清理临时媒体"]="Automatically clean up temporary media",["未保存的录制暂存在本机，并由应用自动清理。"]="Unsaved recordings stay on this device and are cleaned up automatically.",
        ["Provider 配置"]="Provider configuration",["新增"]="Add",["删除"]="Remove",["Provider 名称"]="Provider name",["设为默认 Provider"]="Set as default provider",["OpenAI 兼容"]="OpenAI-compatible",["Provider 类型"]="Provider type",["获取火山模型"]="Get Volcengine models",["API Key（留空则保留现有密钥）"]="API key (leave blank to keep the saved key)",
        ["API Key 与敏感 Custom Header 使用 Windows DPAPI 加密，仅保存在本机当前用户目录。"]="API keys and sensitive custom headers are encrypted with Windows DPAPI and stored only for your Windows account on this device.",
        ["清除已保存密钥"]="Clear saved key",["Custom Headers JSON（高级，敏感值保存时自动加密）"]="Custom headers JSON (advanced; sensitive values are encrypted)",["测试连接"]="Test connection",["使用本机 Hermes"]="Use local Hermes",["本机 Hermes"]="Local Hermes",["普通对话与屏幕对话"]="Text and screen conversations",["重新检测"]="Detect again",["连接测试"]="Test connection",
        ["Hermes Agent / 人格"]="Hermes agent / persona",["Hermes 模型"]="Hermes model",["Hermes 思考程度"]="Hermes reasoning effort",["Agent / 人格"]="Agent / persona",["模型"]="Model",["思考程度"]="Reasoning effort",["回复后自动朗读"]="Read responses aloud",["启用语音输入"]="Enable voice input",["Prompt 出现时自动监听"]="Start listening when the prompt appears",
        ["识别语言"]="Recognition language",["跟随 Windows"]="Use Windows language",["简体中文"]="Simplified Chinese",["英语"]="English",["识别结果只会填入输入框，不会自动发送。"]="Recognized speech is inserted into the prompt and is never sent automatically.",
        ["在本地保存 AI 对话历史"]="Save AI conversation history on this device",["关闭后仍可在本次运行中查看；退出应用后不保留。"]="If disabled, history remains available only until you quit MewuAI.",["媒体默认不永久保存；截图只有明确点击发送后才会上传。"]="Media is not saved permanently by default. Captures are uploaded only when you explicitly send them.",
        ["清空本地对话历史"]="Clear local conversation history",["清理临时媒体"]="Clean up temporary media",["打开数据目录"]="Open data folder",["检查更新 / Check for updates"]="Check for updates",["从 GitHub Releases 获取正式版本。安装包下载后会校验 SHA-256。\nOfficial releases are checked on GitHub and verified with SHA-256."]="MewuAI checks GitHub Releases for official updates and verifies every installer with SHA-256.",
        ["GitHub 开源仓库 / Open-source repository · github.com/abnste/mewu_ai"]="Open-source repository · github.com/abnste/mewu_ai",["许可说明"]="License",["选择标注颜色"]="Choose annotation color",["使用此颜色"]="Use this color",
        ["复制"]="Copy",["保存…"]="Save…",["向左旋转 90°"]="Rotate left 90°",["向右旋转 90°"]="Rotate right 90°",["回到原位"]="Restore original position",["置顶"]="Keep on top",["取消置顶"]="Stop keeping on top",["80% 透明度"]="80% opacity",["100% 不透明度"]="100% opacity",["关闭"]="Close",["播放"]="Play",
        ["保存标注内容"]="Save annotations",["保存带标注版本"]="Save with annotations",["保存干净原件"]="Save original",["稍后 / Later"]="Later",["安装并重启 / Install & restart"]="Install and restart",["更新已准备好 / Update ready"]="Update ready",
        ["快捷键注册失败，可能已被其他应用占用"]="The global shortcut could not be registered because another app may be using it.",["无法开始截图，请重试"]="Could not start capture. Please try again.",["该快捷键可能已被其他应用占用，旧快捷键仍然有效。"]="Another app may be using that shortcut. Your previous shortcut is still active.",["设置已保存，但主界面状态刷新失败。"]="Settings were saved, but the main window could not refresh.",
        ["未检测到本机 Hermes"]="Local Hermes was not detected",["检测失败，请重试"]="Detection failed. Try again.",["正在连接本机 Hermes…"]="Connecting to local Hermes…",["连接超时，请检查 Hermes 配置"]="Connection timed out. Check your Hermes configuration.",["Hermes 未返回可用 Agent / 人格。"]="Hermes did not return an available agent or persona.",["Hermes 未返回可用模型，请先完成 Hermes 模型配置。"]="Hermes did not return an available model. Complete the model setup in Hermes first.",
        ["本地对话历史已清空"]="Local conversation history was cleared.",["本地对话历史清理失败，请稍后重试。"]="Could not clear local conversation history. Please try again.",["无法清理"]="Could not clean up",["暂时无法清理"]="Cleanup unavailable",["临时媒体已清理"]="Temporary media was cleaned up.",["临时媒体清理失败，请关闭正在使用这些文件的程序后重试。"]="Could not clean up temporary media. Close any apps using these files and try again.",
        ["连接成功"]="Connection successful",["服务返回失败状态"]="The service returned an error status.",["AI 连接测试"]="AI connection test",["AI 连接测试失败"]="AI connection test failed",["连接测试已取消或超时，请检查网络与 Provider 地址。"]="The connection test was canceled or timed out. Check your network and provider URL.",["获取模型列表超时，请检查网络后重试。"]="The model request timed out. Check your network and try again.",["火山模型列表"]="Volcengine model list",
        ["至少保留一个 Provider 配置。"]="Keep at least one provider configuration.",["无法保存"]="Could not save",["Provider 配置无效"]="Invalid provider configuration",["默认 Provider 无法使用"]="Default provider unavailable",["清除 API Key"]="Clear API key",["撤销清除"]="Undo clear",["未命名 Provider"]="Unnamed provider",["Provider 名称不能为空"]="Provider name cannot be empty.",
        ["请先连接 Hermes 并选择 Agent / 人格。"]="Connect to Hermes and select an agent or persona first.",["请先连接 Hermes 并选择模型。"]="Connect to Hermes and select a model first.",["未检测到可用的本机 Hermes，请重新检测后再启用。"]="No usable local Hermes installation was detected. Detect it again before enabling Hermes.",["快捷键至少需要 Ctrl、Shift 或 Alt 中的一个修饰键。"]="The shortcut must include Ctrl, Shift, or Alt.",
        ["请先输入 API Key，或在 Custom Headers 中配置认证字段"]="Enter an API key or configure an authentication field in Custom Headers.",["API Key 与认证 Custom Header 不能同时发送。请清除已保存 API Key，或移除认证 Header。"]="An API key and an authentication Custom Header cannot be sent together. Clear the saved API key or remove the authentication header.",
        ["屏幕防捕获不可用，Custom Headers 已隐藏。"]="Screen-capture protection is unavailable, so Custom Headers are hidden.",["屏幕防捕获不可用，API Key 与敏感 Header 已隐藏。"]="Screen-capture protection is unavailable, so the API key and sensitive headers are hidden.",["尚未保存 API Key，可改用认证 Custom Header。"]="No API key is saved. You can use an authentication Custom Header instead.",["已有可用的加密 API Key；输入新值可替换，留空会保留。"]="An encrypted API key is saved. Enter a new key to replace it, or leave this blank to keep it.",["新 API Key 将在保存后替换现有密钥。"]="The new API key will replace the saved key when you save.",["已保存的 API Key 无法读取，请输入新值后保存。"]="The saved API key could not be read. Enter a new key and save.",["保存后将清除现有 API Key；点击“撤销清除”可保留。"]="The saved API key will be removed when you save. Select “Undo clear” to keep it.",
        ["语音输入已在设置中关闭"]="Voice input is disabled in Settings.",["语音输入暂时不可用"]="Voice input is temporarily unavailable.",["正在聆听…"]="Listening…",["正在停止聆听…"]="Stopping voice input…",["已停止聆听"]="Voice input stopped.",["语音已写入"]="Speech inserted into the prompt.",
        ["请先框选截图区域"]="Select a capture region first.",["请先选择区域"]="Select a region first.",["请先配置可用的 AI Provider"]="Configure an available AI provider first.",["当前模型不支持图片理解"]="The current model does not support image understanding.",["当前 Provider 未开启视频理解能力"]="The current provider does not support video understanding.",["当前操作尚未完成 · 按 Esc 可取消"]="The current operation is still running · Press Esc to cancel",["AI 正在分析 · 按 Esc 可取消后再修改区域"]="AI is analyzing · Press Esc to cancel before changing the region",
        ["正在整理回答…"]="Preparing the response…",["正在生成文字回答…按 Esc 可取消"]="Generating a response… Press Esc to cancel",["正在准备文字请求…按 Esc 可取消"]="Preparing the text request… Press Esc to cancel",["正在识别表格结构…按 Esc 可取消"]="Recognizing the table structure… Press Esc to cancel",["没有识别到完整表格，可调整选区后重试"]="No complete table was detected. Adjust the region and try again.",["请先框选包含表格的图片区域"]="Select an image region containing a table first.",["复制表格失败"]="Could not copy the table.",
        ["图片已复制"]="Image copied.",["视频文件已复制"]="Video file copied.",["文字已复制"]="Text copied.",["图片已保存"]="Image saved.",["MP4 原件已保存"]="Original MP4 saved.",["带标注 MP4 已保存"]="Annotated MP4 saved.",["已取消"]="Canceled.",["失败"]="Failed",["确认"]="Confirm",["提交"]="Submit",["拒绝"]="Deny",["继续"]="Continue",["本次会话允许"]="Allow for this session",["始终允许"]="Always allow",["允许一次"]="Allow once",
        ["拖动可连续框选多个区域"]="Drag to select multiple regions",["拖动以添加另一个区域 · 可与现有区域重叠"]="Drag to add another region · Regions may overlap",["框选已结束，可继续操作"]="Selection complete. You can continue.",["框选已中断，请重新拖动选择"]="Selection was interrupted. Drag again to select.",["新建截图区域"]="Create capture region",["移动截图区域"]="Move capture region",["调整截图区域"]="Resize capture region",["删除截图区域"]="Delete capture region",
        ["正在本地识别当前区域…"]="Recognizing text on this device…",["已取消文字识别"]="Text recognition canceled.",["可跨行拖动选择文字，Ctrl+C 复制"]="Drag across lines to select text, then press Ctrl+C to copy",["可跨行拖动选择译文，Ctrl+C 复制"]="Drag across lines to select the translation, then press Ctrl+C to copy",["原位译文 · 可拖选复制"]="In-place translation · Drag to select and copy",["已取消翻译"]="Translation canceled.",
        ["覆盖层防捕获不可用，无法安全生成长截图"]="Capture protection is unavailable, so scrolling capture cannot run safely.",["滚轮向下控制截取长度 · 已截内容会在旁边向上拼接"]="Scroll down to control the capture length · Captured content grows upward in the preview",["请把鼠标放在长截图区域内滚动"]="Move the pointer inside the scrolling-capture region.",["正在等待页面滚动完成…"]="Waiting for the page to settle…",["没有检测到新的滚动内容；可能已到底或滚动过快"]="No new content was detected. You may have reached the end or scrolled too quickly.",["已向上滚动；继续向下滚动才会追加长图"]="Scrolled up. Scroll down to append more content.",["已达到 24 段安全上限，请点击完成"]="The 24-segment safety limit has been reached. Select Finish.",["已达到长图像素安全上限，请点击完成"]="The scrolling-capture pixel limit has been reached. Select Finish.",["没有采集到可用画面"]="No usable content was captured.",["已取消长截图"]="Scrolling capture canceled.",
        ["原位标注中 · 颜色统一作用于画笔、形状、文字和序号"]="Annotating in place · The selected color applies to pens, shapes, text, and numbers",["视频原位标注中 · 手工标注将贯穿整个视频"]="Annotating video in place · Manual annotations remain visible throughout the video",["点击选择标注 · 拖动移动 · Delete 删除"]="Select an annotation, drag to move it, or press Delete",["拖过标注即可擦除 · 支持笔迹、形状、文字、序号和马赛克"]="Drag across an annotation to erase it · Supports strokes, shapes, text, numbers, and pixelation",["没有命中标注对象"]="No annotation was hit.",["请先点击选择要删除的标注"]="Select an annotation before deleting it.",
        ["暂无历史对话 / No conversation yet"]="No conversation yet",["未收到 AI 回复 / No AI response"]="No AI response",["用户 / You"]="You",["查看提问与历史 / Prompt & history"]="Prompt & history",["进行中"]="In progress",["处理中…"]="Working…",["正在生成回答… / Generating response…"]="Generating response…",["Hermes 等待你的确认"]="Hermes is waiting for your confirmation",["Hermes 已收到选择，继续处理中…"]="Hermes received your choice and is continuing…"
    };

    private static readonly (Regex Pattern,string Replacement)[] EnglishPatterns=
    [
        (new Regex("^已采集 1 段$",RegexOptions.CultureInvariant),"1 segment captured"),
        (new Regex("^已采集 (\\d+) 段 · (\\d+)px$",RegexOptions.CultureInvariant),"Captured $1 segments · $2 px"),
        (new Regex("^已采集 (\\d+) 段$",RegexOptions.CultureInvariant),"Captured $1 segments"),
        (new Regex("^截图 (\\d+) × (\\d+)$",RegexOptions.CultureInvariant),"Capture $1 × $2"),
        (new Regex("^(\\d+) 秒$",RegexOptions.CultureInvariant),"$1 seconds"),
        (new Regex("^(\\d+) 天$",RegexOptions.CultureInvariant),"$1 days"),
        (new Regex("^已获取 (\\d+) 个$",RegexOptions.CultureInvariant),"Loaded $1"),
        (new Regex("^已识别 (\\d+) 个表格 · 点击回答上方的“复制表格”$",RegexOptions.CultureInvariant),"Recognized $1 tables · Select “Copy table” above the response"),
        (new Regex("^已复制 (\\d+) 个表格 · Excel 可直接粘贴，文本框为 Markdown，桌面为 PNG$",RegexOptions.CultureInvariant),"Copied $1 tables · Paste editable cells into Excel, Markdown into text fields, or a PNG onto the desktop"),
        (new Regex("^已检测 · (.+)$",RegexOptions.CultureInvariant),"Detected · $1"),
        (new Regex("^已是最新版 (v[^ ]+)(?: / You're up to date)?$",RegexOptions.CultureInvariant),"You're up to date ($1)"),
        (new Regex("^(v[^ ]+) 已下载并通过校验(?: / Downloaded and verified)?$",RegexOptions.CultureInvariant),"$1 downloaded and verified")
    ];

    internal static AppLanguage Language { get; private set; }=ResolveLanguage(CultureInfo.CurrentUICulture);
    internal static bool IsEnglish=>Language==AppLanguage.English;
    internal static string CultureName=>Language==AppLanguage.SimplifiedChinese?"zh-CN":"en-US";

    internal static void Initialize(string? preference,CultureInfo? systemUiCulture=null)
    {
        Language=ResolveLanguagePreference(preference,systemUiCulture??CultureInfo.CurrentUICulture);
        FrameworkElement.LanguageProperty.OverrideMetadata(typeof(FrameworkElement),new FrameworkPropertyMetadata(XmlLanguage.GetLanguage(CultureName)));
        if(IsEnglish)EventManager.RegisterClassHandler(typeof(FrameworkElement),FrameworkElement.LoadedEvent,new RoutedEventHandler(OnElementLoaded));
    }

    internal static string T(string chinese,string english)=>IsEnglish?english:chinese;
    internal static string Format(string chinese,string english,params object?[] args)=>string.Format(CultureInfo.CurrentCulture,IsEnglish?english:chinese,args);

    internal static string TranslateUiText(string? value)=>TranslateUiText(value,Language);

    internal static string TranslateUiText(string? value,AppLanguage language)
    {
        if(language!=AppLanguage.English||string.IsNullOrEmpty(value))return value??string.Empty;
        if(English.TryGetValue(value,out var translated))return translated;
        var hermesCounts=Regex.Match(value,"^连接正常 · (\\d+) 个 Agent · (\\d+) 个模型$",RegexOptions.CultureInvariant);
        if(hermesCounts.Success)
        {
            var agents=int.Parse(hermesCounts.Groups[1].Value,CultureInfo.InvariantCulture);
            var models=int.Parse(hermesCounts.Groups[2].Value,CultureInfo.InvariantCulture);
            return $"Connected · {agents} agent{(agents==1?string.Empty:"s")} · {models} model{(models==1?string.Empty:"s")}";
        }
        foreach(var (pattern,replacement) in EnglishPatterns)if(pattern.IsMatch(value))return pattern.Replace(value,replacement);
        return value;
    }

    internal static AppLanguage ResolveLanguage(CultureInfo culture)=>string.Equals(culture.TwoLetterISOLanguageName,"zh",StringComparison.OrdinalIgnoreCase)?AppLanguage.SimplifiedChinese:AppLanguage.English;
    internal static AppLanguage ResolveLanguagePreference(string? preference,CultureInfo systemUiCulture)=>preference?.Trim() switch
    {
        "zh-CN"=>AppLanguage.SimplifiedChinese,
        "en-US"=>AppLanguage.English,
        _=>ResolveLanguage(systemUiCulture)
    };

    private static void OnElementLoaded(object sender,RoutedEventArgs args)
    {
        // Loaded is routed through the owning window.  Using only `sender`
        // localizes the window chrome but misses the originating child (most
        // visible labels in code-built pages).  Observe the true source so
        // controls created later, including lazily realized tab content, are
        // localized as soon as they enter the visual tree.
        var element=args.OriginalSource as FrameworkElement??sender as FrameworkElement;
        if(element is null||IsExcluded(element))return;
        UiTextWatcher.Attach(element);
        if(element is Window)AttachExistingTree(element);
    }

    private static void AttachExistingTree(DependencyObject root)
    {
        var pending=new Stack<DependencyObject>();var visited=new HashSet<DependencyObject>();pending.Push(root);
        while(pending.Count>0)
        {
            var current=pending.Pop();if(!visited.Add(current)||IsExcluded(current))continue;
            if(current is FrameworkElement framework)UiTextWatcher.Attach(framework);
            foreach(var child in LogicalTreeHelper.GetChildren(current).OfType<DependencyObject>())pending.Push(child);
            if(current is not System.Windows.Media.Visual and not System.Windows.Media.Media3D.Visual3D)continue;
            var count=System.Windows.Media.VisualTreeHelper.GetChildrenCount(current);
            for(var index=0;index<count;index++)pending.Push(System.Windows.Media.VisualTreeHelper.GetChild(current,index));
        }
    }

    private static bool IsExcluded(DependencyObject element)
    {
        for(DependencyObject? current=element;current is not null;current=current is FrameworkElement framework?framework.Parent:LogicalTreeHelper.GetParent(current))if(GetExcludeFromLocalization(current))return true;
        return false;
    }

    internal static readonly DependencyProperty ExcludeFromLocalizationProperty=DependencyProperty.RegisterAttached("ExcludeFromLocalization",typeof(bool),typeof(LocalizationService),new PropertyMetadata(false));
    internal static void SetExcludeFromLocalization(DependencyObject element,bool value)=>element.SetValue(ExcludeFromLocalizationProperty,value);
    internal static bool GetExcludeFromLocalization(DependencyObject element)=>(bool)element.GetValue(ExcludeFromLocalizationProperty);

    private sealed class UiTextWatcher
    {
        private static readonly DependencyProperty ObservedProperty=DependencyProperty.RegisterAttached("Observed",typeof(bool),typeof(UiTextWatcher),new PropertyMetadata(false));
        private readonly FrameworkElement _element;
        private readonly List<(DependencyPropertyDescriptor Descriptor,EventHandler Handler)> _handlers=[];
        private bool _updating;
        private UiTextWatcher(FrameworkElement element){_element=element;Observe();element.Unloaded+=OnUnloaded;}

        internal static void Attach(FrameworkElement element){if((bool)element.GetValue(ObservedProperty))return;element.SetValue(ObservedProperty,true);_ = new UiTextWatcher(element);}

        private void Observe()
        {
            if(_element is TextBlock)Add(TextBlock.TextProperty,typeof(TextBlock));
            if(_element is ContentControl)Add(ContentControl.ContentProperty,typeof(ContentControl));
            if(_element is HeaderedContentControl)Add(HeaderedContentControl.HeaderProperty,typeof(HeaderedContentControl));
            if(_element is HeaderedItemsControl)Add(HeaderedItemsControl.HeaderProperty,typeof(HeaderedItemsControl));
            if(_element is Window)Add(Window.TitleProperty,typeof(Window));
            Add(ToolTipService.ToolTipProperty,typeof(FrameworkElement));Add(AutomationProperties.NameProperty,typeof(DependencyObject));
        }

        private void Add(DependencyProperty property,Type owner)
        {
            var descriptor=DependencyPropertyDescriptor.FromProperty(property,owner);if(descriptor is null)return;EventHandler handler=(_,_)=>Translate(property);descriptor.AddValueChanged(_element,handler);_handlers.Add((descriptor,handler));Translate(property);
        }

        private void Translate(DependencyProperty property)
        {
            if(_updating||_element.GetValue(property) is not string source)return;var translated=TranslateUiText(source);if(string.Equals(source,translated,StringComparison.Ordinal))return;try{_updating=true;_element.SetCurrentValue(property,translated);}finally{_updating=false;}
        }

        private void OnUnloaded(object sender,RoutedEventArgs args)
        {
            foreach(var (descriptor,handler) in _handlers)descriptor.RemoveValueChanged(_element,handler);_handlers.Clear();_element.Unloaded-=OnUnloaded;_element.ClearValue(ObservedProperty);
        }
    }
}

internal static class LocalizedMessageBox
{
    internal static MessageBoxResult Show(string message,string caption)=>MessageBox.Show(LocalizationService.TranslateUiText(message),LocalizationService.TranslateUiText(caption));
    internal static MessageBoxResult Show(string message,string caption,MessageBoxButton buttons,MessageBoxImage icon)=>MessageBox.Show(LocalizationService.TranslateUiText(message),LocalizationService.TranslateUiText(caption),buttons,icon);
    internal static MessageBoxResult Show(Window owner,string message,string caption)=>MessageBox.Show(owner,LocalizationService.TranslateUiText(message),LocalizationService.TranslateUiText(caption));
    internal static MessageBoxResult Show(Window owner,string message,string caption,MessageBoxButton buttons)=>MessageBox.Show(owner,LocalizationService.TranslateUiText(message),LocalizationService.TranslateUiText(caption),buttons);
    internal static MessageBoxResult Show(Window owner,string message,string caption,MessageBoxButton buttons,MessageBoxImage icon)=>MessageBox.Show(owner,LocalizationService.TranslateUiText(message),LocalizationService.TranslateUiText(caption),buttons,icon);
}
