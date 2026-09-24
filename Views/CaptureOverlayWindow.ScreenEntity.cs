// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Diagnostics;
using System.Text;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using Microsoft.Win32;
using mewu_ai_Assistant.OCR;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    // 屏幕实体（链接/邮箱）识别：选区完成后提取其文本快照（窗口吸附走 UIA，
    // 普通拖选走本地 OCR），识别出的链接/邮箱会显示在选区旁的悬浮条上，
    // 点击即可打开链接或直接撰写并发送邮件——无需先与 AI 对话。
    private const int MaxScreenEntityTextLength=12000;
    private readonly HashSet<SelectionItem> _screenTextInFlight=new();

    private void TryBeginScreenEntityScan(SelectionItem item)
    {
        if(_closed||_recordingMode||_drawingMode||_longCaptureMode)return;
        if(item.IsImplicit||item.VideoPath is not null||item.SnapshotText is not null)return;
        if(!_screenTextInFlight.Add(item))return;
        _=LoadScreenEntityTextAsync(item);
    }

    private async Task LoadScreenEntityTextAsync(SelectionItem item)
    {
        try
        {
            string? text=null;
            if(item.SnapshotTarget is { } target)
            {
                // UIA 文本提取在独立工作进程中进行，不会阻塞界面。
                var document=await ApplicationSnapshotProcess.ReadAsync(target,CancellationToken.None).ConfigureAwait(false);
                text=document.Text;
            }
            else
            {
                // 尚未离开 UI 线程：先同步取图，再做本地 OCR。
                var image=RenderSelectionImage(item,false,false,false);
                var recognized=await new WindowsOcrService().RecognizeAsync(image,CancellationToken.None).ConfigureAwait(false);
                text=recognized?.Text;
            }
            if(_closed)return;
            await Dispatcher.InvokeAsync(()=>
            {
                if(_closed||!_selections.Contains(item))return;
                var trimmed=(text??string.Empty).Trim();
                item.SnapshotText=trimmed.Length>MaxScreenEntityTextLength?trimmed[..MaxScreenEntityTextLength]:trimmed;
                UpdateScreenEntityBar(item);
            }).Task.ConfigureAwait(false);
        }
        catch(OperationCanceledException){}
        catch(Exception ex){new PrivacyLogger().Info("ScreenEntityText",ex.GetType().Name);}
        finally{_screenTextInFlight.Remove(item);}
    }

    private void ShowPhoneActionsForText(string? text)
        => ShowPhoneActions(PhoneNumberService.Find(text ?? string.Empty));

    private void HideScreenEntityBar()=>ScreenEntityBar.Visibility=Visibility.Collapsed;

    private bool _scraplingCrawlInFlight;

    /// <summary>用本机 Scrapling 抓取 URL 正文并弹出结果窗口（后台执行，抓取中状态栏提示）。
    /// 未安装 Scrapling 时先走内置基础抓取（纯 HTTP、零依赖）：静态页面直接出结果；
    /// 需要 JS 渲染/被反爬拦截（微信公众号、知乎）时再引导用户一键安装——mewuAI 自动
    /// 调用 py/python 启动器创建 venv 并 pip install scrapling + scrapling install
    /// 下载隐身浏览器。</summary>
    private async Task CrawlUrlWithScraplingAsync(string url)
    {
        if(_scraplingCrawlInFlight)
        {
            PromptStatus.Text=L("正在抓取中，请稍候…","A crawl is still running; please wait…");
            return;
        }
        var scraplingDir=ScraplingCrawlService.Locate(_host.Settings);
        if(scraplingDir is null)
        {
            // 未装 Scrapling：先试内置基础抓取（无需任何安装）。
            PromptStatus.Text=L("未检测到 Scrapling，先用内置基础抓取（零依赖）…","Scrapling not found; trying the built-in basic fetch (zero dependencies)…");
            ScraplingCrawlService.CrawlResult? basic=null;
            try
            {
                basic=await Task.Run(()=>BasicHttpCrawlService.TryCrawlAsync(url,CancellationToken.None)).ConfigureAwait(true);
            }
            catch(Exception ex){new PrivacyLogger().Info("BasicHttpCrawlUi",ex.GetType().Name);}
            if(basic is not null)
            {
                ShowCrawlResult(basic);
                PromptStatus.Text=L($"基础抓取完成（{basic.Text.Length} 字）。动态页面（微信/知乎等）建议安装 Scrapling 获得完整正文。",$"Basic fetch done ({basic.Text.Length} chars). Install Scrapling for full text of dynamic pages (WeChat/Zhihu…).");
                return;
            }
            // 基础抓取拿不到正文（需要渲染/被拦截）：弹窗让用户一键安装。
            var choice=PromptMissingScraplingChoice(url);
            if(choice==ScraplingChoice.Cancel)return;
            if(choice==ScraplingChoice.CopySteps)
            {
                ClipboardService.TrySetText(ScraplingCrawlService.ManualInstallSteps(),out _);
                PromptStatus.Text=L("安装命令已复制到剪贴板。","Install commands copied to clipboard.");
                return;
            }
            // 一键安装
            await InstallScraplingThenCrawlAsync(url);
            return;
        }
        await DoCrawlAsync(url,scraplingDir);
    }

    private enum ScraplingChoice{OneClickInstall,CopySteps,Cancel}

    private ScraplingChoice PromptMissingScraplingChoice(string url)
    {
        var isWeixin=url.IndexOf("mp.weixin.qq.com",StringComparison.OrdinalIgnoreCase)>=0;
        var title=L("该页面需要 Scrapling 隐身浏览器","This page needs the Scrapling stealth browser");
        var msg=isWeixin
            ?L(
                "内置基础抓取拿不到这个页面的正文：微信公众号需要隐身浏览器渲染。mewuAI 可一键帮你安装 Scrapling（约 30 MB 下载）。\n\n点击\"安装\" → 自动调用 py/python 启动器创建 venv、pip install scrapling、下载隐身浏览器。",
                "The built-in basic fetch could not get this page's content: WeChat articles need a stealth browser to render. MewuAI can install Scrapling for you in one click (~30 MB download).\n\nClick \"Install\" to let MewuAI run py/python to create a venv, pip install scrapling and download the stealth browser.")
            :L(
                "内置基础抓取拿不到这个页面的正文：它需要隐身浏览器渲染（或被站点反爬拦截）。mewuAI 可一键帮你安装 Scrapling。\n\n点击\"安装\" → 自动调用 py/python 启动器创建 venv、pip install scrapling、下载隐身浏览器。",
                "The built-in basic fetch could not get this page's content: it needs a stealth browser to render (or was blocked by anti-bot protection). MewuAI can install Scrapling for you in one click.\n\nClick \"Install\" to let MewuAI run py/python to create a venv, pip install scrapling and download the stealth browser.");
        var installText=L("一键安装（约 30MB）","One-click install (~30 MB)");
        var stepsText=L("复制手动安装步骤","Copy manual steps");
        var cancelText=L("取消","Cancel");
        var result=MewuDialogWindow.ShowChoice(this,title,msg,installText,stepsText,cancelText);
        return result switch
        {
            MewuDialogResult.Primary=>ScraplingChoice.OneClickInstall,
            MewuDialogResult.Secondary=>ScraplingChoice.CopySteps,
            _=>ScraplingChoice.Cancel,
        };
    }

    /// <summary>运行一键安装（带进度流），完成后立即抓取。</summary>
    private async Task InstallScraplingThenCrawlAsync(string url)
    {
        _scraplingCrawlInFlight=true;
        var progress=new Progress<string>(line=>Dispatcher.Invoke(()=>PromptStatus.Text=line));
        try
        {
            var result=await ScraplingCrawlService.EnsureInstalledAsync(progress,CancellationToken.None).ConfigureAwait(true);
            if(!result.Success)
            {
                MewuDialogWindow.ShowMessage(this,
                    L("Scrapling 安装未成功","Scrapling install did not complete"),
                    string.Format(System.Globalization.CultureInfo.CurrentCulture,
                        L("安装失败：{0}\\n\\n已在 MewuAI 日志中记录详细 stderr。可改用\"复制手动安装步骤\"自行安装。","result={0}\\n\\nDetailed stderr was written to the MewuAI log. You can also use \"Copy manual steps\" to install it yourself."),
                        result.Message));
                return;
            }
            PromptStatus.Text=L("Scrapling 安装完成，开始抓取…","Scrapling installed; starting crawl…");
            await DoCrawlAsync(url,result.InstallDir).ConfigureAwait(true);
        }
        finally{_scraplingCrawlInFlight=false;}
    }

    /// <summary>实际的抓取流程（Scrapling 已就绪后调用）。</summary>
    private async Task DoCrawlAsync(string url,string scraplingDir)
    {
        _scraplingCrawlInFlight=true;
        PromptStatus.Text=L("Scrapling 正在抓取网页内容（先快速请求，必要时隐身浏览器渲染）…","Scrapling is fetching the page (quick request first, stealth browser if needed)…");
        try
        {
            var result=await Task.Run(()=>ScraplingCrawlService.CrawlAsync(url,scraplingDir,CancellationToken.None)).ConfigureAwait(true);
            ShowCrawlResult(result);
        }
        catch(Exception ex)
        {
            PromptStatus.Text=L($"抓取失败：{ex.Message}",$"Crawl failed: {ex.Message}");
            ScreenEntityBar.Visibility=Visibility.Visible;
            new PrivacyLogger().Info("ScraplingCrawl",ex.GetType().Name);
        }
        finally{_scraplingCrawlInFlight=false;}
    }

    /// <summary>展示抓取结果：自动落盘、可选另存、复制剪贴板、弹出结果窗口。
    /// 若链接被自动纠正（OCR 易混字符），状态栏和保存文件都会注明。</summary>
    private void ShowCrawlResult(ScraplingCrawlService.CrawlResult result)
    {
        new PrivacyLogger().Info("ScraplingUiResult",$"length={result.Text.Length};parameterError={result.Text.Contains("Parameter error",StringComparison.OrdinalIgnoreCase)};corrected={result.CorrectedFrom is not null};engine={result.Engine??"Scrapling"}");
        // 结果自动落盘：%LOCALAPPDATA%\MewuAI\Crawls\<时间>-<标题>.md
        var savedPath=SaveCrawlResult(result);
        var chosenPath=PromptSaveCrawlResult(result,savedPath);
        if(!string.IsNullOrWhiteSpace(chosenPath))savedPath=chosenPath;
        var copied=ClipboardService.TrySetText(result.Text,out _);
        var correctedNote=result.CorrectedFrom is not null
            ?L("（已自动纠正识别错的链接字符）","(misread link characters auto-corrected)")
            :string.Empty;
        PromptStatus.Text=copied
            ?L($"抓取完成，正文已复制到剪贴板：{TruncateForStatus(result.Title,40)}（{result.Text.Length} 字）{correctedNote} · 文件：{savedPath}",$"Fetched and copied to clipboard: {TruncateForStatus(result.Title,40)} ({result.Text.Length} chars) {correctedNote} · file: {savedPath}")
            :L($"抓取完成：{TruncateForStatus(result.Title,40)}（{result.Text.Length} 字）{correctedNote} · 文件：{savedPath}",$"Fetched: {TruncateForStatus(result.Title,40)} ({result.Text.Length} chars) {correctedNote} · file: {savedPath}");
        var window=new ScraplingCrawlResultWindow(_closed?null:this,result,savedPath);
        window.Show();
    }

    /// <summary>把抓取结果保存为 Markdown 文件（标题 + 链接 + 时间 + 正文），返回保存路径；失败返回 null。</summary>
    private static string? SaveCrawlResult(ScraplingCrawlService.CrawlResult result)
    {
        try
        {
            var folder=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Crawls");
            Directory.CreateDirectory(folder);
            var invalid=Path.GetInvalidFileNameChars();
            var safeTitle=new string((result.Title.Length>0?result.Title:"untitled").Select(ch=>invalid.Contains(ch)?'_':ch).ToArray());
            if(safeTitle.Length>60)safeTitle=safeTitle[..60].TrimEnd();
            var path=Path.Combine(folder,$"{DateTime.Now:yyyyMMdd-HHmmss}-{safeTitle}.md");
            File.WriteAllText(path,CrawlResultMarkdown(result),new UTF8Encoding(false));
            return path;
        }
        catch(Exception ex){new PrivacyLogger().Info("ScraplingCrawlSave",ex.GetType().Name);return null;}
    }

    /// <summary>生成保存到 Markdown 的内容：注明抓取引擎（Scrapling / 内置基础抓取）
    /// 与链接自动纠正信息。</summary>
    private static string CrawlResultMarkdown(ScraplingCrawlService.CrawlResult result)
    {
        var engine=string.IsNullOrEmpty(result.Engine)?"Scrapling":result.Engine;
        var sb=new StringBuilder()
            .AppendLine($"# {result.Title}")
            .AppendLine()
            .AppendLine($"> 来源：{result.Url}");
        if(result.CorrectedFrom is not null)
            sb.AppendLine($"> 注：圈选识别到的链接 {result.CorrectedFrom} 含易混字符（I/l/1、O/0），已自动纠正。");
        return sb.AppendLine($"> 抓取时间：{DateTime.Now:yyyy-MM-dd HH:mm:ss}（{engine}）")
            .AppendLine()
            .AppendLine(result.Text)
            .ToString();
    }

    private string? PromptSaveCrawlResult(ScraplingCrawlService.CrawlResult result,string? fallbackPath)
    {
        try
        {
            var dialog=new SaveFileDialog
            {
                Title=L("保存抓取结果","Save crawl result"),
                Filter="Markdown (*.md)|*.md|文本文件 (*.txt)|*.txt|所有文件 (*.*)|*.*",
                FileName=Path.GetFileName(fallbackPath??$"{DateTime.Now:yyyyMMdd-HHmmss}-crawl.md"),
                AddExtension=true,
                DefaultExt=".md",
                OverwritePrompt=true
            };
            if(dialog.ShowDialog(this)==true)
            {
                File.WriteAllText(dialog.FileName,CrawlResultMarkdown(result),new UTF8Encoding(false));
                return dialog.FileName;
            }
        }
        catch(Exception ex){new PrivacyLogger().Info("ScraplingCrawlSaveAs",ex.GetType().Name);}
        return fallbackPath;
    }

    private static string TruncateForStatus(string value,int max)=>value.Length<=max?value:value[..max]+"…";

    /// <summary>根据选区文本里的实体刷新悬浮动作条；识别失败或无实体时隐藏。</summary>
    private void UpdateScreenEntityBar(SelectionItem item)
    {
        var entities=ScreenEntityRecognitionService.Extract(item.SnapshotText);
        if(entities.Count==0||_closed||!_selections.Contains(item)){HideScreenEntityBar();return;}
        ScreenEntityBarContent.Children.Clear();
        var url=entities.FirstOrDefault(entity=>entity.Type==ScreenEntityType.Url);
        if(url is not null)
        {
            var host=Uri.TryCreate(url.Value,UriKind.Absolute,out var parsed)?parsed.Host:url.Value;
            ScreenEntityBarContent.Children.Add(EntityBarButton(
                string.IsNullOrEmpty(host)?L("打开链接","Open link"):string.Format(System.Globalization.CultureInfo.CurrentCulture,L("打开 {0}","Open {0}"),host),
                url.Value,url.Value,()=>{
                    ScreenEntityMcpService.OpenUrl(url.Value);
                    // 打开网站后立即收起识屏浮层，避免冻结帧遮挡浏览器导致卡顿。
                    DismissOverlayAfterExternalAction();
                }));
            ScreenEntityBarContent.Children.Add(EntityBarButton(
                L("复制链接","Copy link"),
                url.Value,L("复制 URL 到剪贴板","Copy the URL to the clipboard"),()=>{
                    ClipboardService.TrySetText(url.Value,out _);
                    PromptStatus.Text=L("链接已复制。","Link copied.");
                }));
            // “爬取内容”：抓取微信公众号文章等页面正文。未安装 Scrapling 时先走
            // 内置基础抓取（零依赖）；拿不到正文（需要渲染/被反爬拦截）再引导一键安装。
            var scraplingReady=ScraplingCrawlService.Locate(_host.Settings) is not null;
            var crawlLabel=scraplingReady
                ?L("Scrapling 爬取","Scrapling crawl")
                :L("爬取内容","Crawl content");
            var crawlTip=scraplingReady
                ?L("用本机 Scrapling 抓取该网页正文（如微信公众号文章），结果自动保存到本机 Crawls 文件夹","Fetch the page content with the local Scrapling crawler (e.g. WeChat articles); results are auto-saved to the local Crawls folder")
                :L("未检测到 Scrapling：先用内置基础抓取（无需安装）；动态页面（微信/知乎等）会引导一键安装 Scrapling（约 30 MB）","Scrapling not detected: the built-in basic fetch runs first (no install); dynamic pages (WeChat/Zhihu…) will guide a one-click Scrapling install (~30 MB)");
            ScreenEntityBarContent.Children.Add(EntityBarButton(
                crawlLabel,
                url.Value,crawlTip,()=>{
                    _=CrawlUrlWithScraplingAsync(url.Value);
                }));
        }
        // 每个识别到的邮箱都提供撰写入口；QQ/foxmail 地址优先走 QQ 邮箱 MCP，
        // 网易后缀（163/126/yeah 等）优先走网易 SMTP，其余按可用渠道回退。
        var email=entities.FirstOrDefault(entity=>entity.Type==ScreenEntityType.Email&&entity.MailProvider=="qq")
            ??entities.FirstOrDefault(entity=>entity.Type==ScreenEntityType.Email&&entity.MailProvider=="netease")
            ??entities.FirstOrDefault(entity=>entity.Type==ScreenEntityType.Email);
        if(email is not null)
        {
            ScreenEntityBarContent.Children.Add(EntityBarButton(
                string.Format(System.Globalization.CultureInfo.CurrentCulture,L("发邮件给 {0}","Email {0}"),email.Value),
                email.Value,$"{email.DisplayProvider} · {email.Value}",()=>ComposeScreenEntityMail(email)));
        }
        var phone=entities.FirstOrDefault(entity=>entity.Type==ScreenEntityType.Phone);
        if(phone is not null)
        {
            ScreenEntityBarContent.Children.Add(EntityBarButton(L("拨号","Call"),phone.Value,L("拨打电话号码","Dial phone number"),()=>
            {
                try{Process.Start(new ProcessStartInfo($"tel:{phone.Value}"){UseShellExecute=true});PromptStatus.Text=L("已请求系统拨号。","Dial request sent to the system.");}
                catch(Exception ex){new PrivacyLogger().Info("PhoneDial",ex.GetType().Name);PromptStatus.Text=L("系统不支持拨号，请复制号码。","Dialing is unavailable; copy the number instead.");}
            }));
            ScreenEntityBarContent.Children.Add(EntityBarButton(L("复制号码","Copy number"),phone.Value,L("复制电话号码","Copy phone number"),()=>{ClipboardService.TrySetText(phone.Value,out _);PromptStatus.Text=L("电话号码已复制。","Phone number copied.");}));
            ScreenEntityBarContent.Children.Add(EntityBarButton(L("发短信","SMS"),phone.Value,L("向电话号码发送短信","Send an SMS to this number"),()=>Services.PhoneNumberService.SendSms(phone.Value)));
            ScreenEntityBarContent.Children.Add(EntityBarButton(L("查询归属","Lookup"),phone.Value,L("在 IP138 查询号码归属","Look up the phone number on IP138"),()=>ScreenEntityMcpService.OpenUrl($"https://www.ip138.com/mobile.asp?mobile={Uri.EscapeDataString(phone.Value)}&action=mobile")));
            ShowPhoneActionsForText(item.SnapshotText);
        }
        else
        {
            ShowPhoneActions([]);
        }
        var remaining=entities.Where(entity=>!ReferenceEquals(entity,url)&&!ReferenceEquals(entity,email)).ToArray();
        if(remaining.Length>0)
        {
            var more=new TextBlock{Text=$"+{remaining.Length}",Foreground=new SolidColorBrush(Color.FromRgb(101,116,138)),FontSize=12,VerticalAlignment=VerticalAlignment.Center,Margin=new Thickness(8,0,2,0),
                ToolTip=string.Join("\n",remaining.Select(entity=>entity.Value))};
            ScreenEntityBarContent.Children.Add(more);
        }
        ScreenEntityBar.Visibility=Visibility.Visible;
        PositionFloatingBar(ScreenEntityBar,item);
    }

    private Button EntityBarButton(string text,string name,string toolTip,Action click)
    {
        var button=new Button{Content=text,ToolTip=toolTip,Padding=new Thickness(10,4,10,4)};
        System.Windows.Automation.AutomationProperties.SetName(button,name);
        button.SetResourceReference(StyleProperty,"ReferenceChipButton");
        button.Cursor=Cursors.Hand;
        button.Click+=(_,_)=>click();
        return button;
    }

    /// <summary>识别到邮箱后的发件入口：按地址后缀与已配置的渠道选择撰写窗口
    /// （QQ 邮箱 MCP 两阶段确认 / 网易邮箱 SMTP），均未配置时复制地址并打开网页邮箱。</summary>
    private void ComposeScreenEntityMail(ScreenEntity entity)
    {
        var settings=_host.Settings;
        var qqAuthorized=settings.QqMailMcpEnabled&&QqMailMcpService.ReadCachedToken() is not null;
        var neteaseReady=NetEaseMailService.IsConfigured(settings)||NetEaseMailWebMcpService.HasSession();
        if(qqAuthorized&&neteaseReady)
        {
            var choice=MewuDialogWindow.ShowChoice(this,LocalizationService.T("选择发件邮箱","Choose sending account"),LocalizationService.T("检测到 QQ 邮箱和网易邮箱均已授权，请选择本次发送使用的邮箱。","Both QQ Mail and NetEase Mail are authorized. Choose the account for this message."),LocalizationService.T("使用 QQ 邮箱","Use QQ Mail"),LocalizationService.T("使用网易邮箱","Use NetEase Mail"));
            if(choice==MewuDialogResult.Primary)
            {
                OpenQqCompose(entity);return;
            }
            if(choice==MewuDialogResult.Secondary)
            {
                OpenNetEaseCompose(entity);return;
            }
            else return;
        }
        var preferNetEase=entity.MailProvider=="netease"&&neteaseReady;
        if(preferNetEase||!qqAuthorized&&neteaseReady)
        {
            OpenNetEaseCompose(entity);
            return;
        }
        if(qqAuthorized)
        {
            OpenQqCompose(entity);
            return;
        }
        ScreenEntityMcpService.OpenMailProvider(entity);
        PromptStatus.Text=L("已复制邮箱地址并打开网页邮箱。要在本应用内直接发送，请到 设置 → MCP 配置 QQ 邮箱或网易邮箱。","Address copied and webmail opened. To send directly from this app, configure QQ Mail or NetEase Mail under Settings → MCP.");
    }

    private void OpenQqCompose(ScreenEntity entity)
    {
        var qq=new MailComposeWindow(this,entity.Value,LocalizationService.T("通过 QQ 邮箱发送","Send via QQ Mail"),(subject,body,token)=>SendViaQqMailAsync(entity.Value,subject,body,token)){OnSent=DismissOverlayAfterExternalAction};
        qq.Show();
    }

    private void OpenNetEaseCompose(ScreenEntity entity)
    {
        var netease=new MailComposeWindow(this,entity.Value,LocalizationService.T("通过网易邮箱发送","Send via NetEase Mail"),(subject,body,token)=>SendViaNetEaseAsync(entity.Value,subject,body,token)){OnSent=DismissOverlayAfterExternalAction};
        netease.Show();
    }

    private async Task<(bool Success,string Message)> SendViaQqMailAsync(string address,string subject,string body,CancellationToken token)
    {
        var draft=new QqMailDraft([(address,null)],Array.Empty<(string,string?)>(),subject,body);
        var outcome=await QqMailSendService.DeliverAsync(draft,_host.Settings,
            summary=>Task.FromResult(Dispatcher.Invoke(()=>MewuDialogWindow.ShowChoice(this,LocalizationService.T("QQ 邮箱发送确认","QQ Mail send confirmation"),summary,LocalizationService.T("确认发送","Confirm send"),string.Empty)==MewuDialogResult.Primary)),
            token).ConfigureAwait(true);
        var success=outcome.Contains("已发送",StringComparison.Ordinal)||outcome.Contains("was sent",StringComparison.Ordinal);
        return (success,outcome);
    }

    private async Task<(bool Success,string Message)> SendViaNetEaseAsync(string address,string subject,string body,CancellationToken token)
    {
        try
        {
            var draft=new QqMailDraft([(address,null)],Array.Empty<(string,string?)>(),subject,body);
            var outcome=await QqMailSendService.DeliverAsync(draft,_host.Settings,
                summary=>Task.FromResult(Dispatcher.Invoke(()=>MewuDialogWindow.ShowChoice(this,LocalizationService.T("网易邮箱发送确认","NetEase Mail send confirmation"),summary,LocalizationService.T("确认发送","Confirm send"),string.Empty)==MewuDialogResult.Primary)),
                token,true).ConfigureAwait(true);
            var success=outcome.Contains("已发送",StringComparison.Ordinal)||outcome.Contains("was sent",StringComparison.Ordinal);
            return (success,outcome);
        }
        catch(Exception ex)
        {
            return (false,ex.Message);
        }
    }
}

/// <summary>Scrapling 抓取结果展示窗：标题/URL/正文，支持一键复制全文、打开保存位置。
/// 抓取结果已自动保存到 %LOCALAPPDATA%\MewuAI\Crawls\（Markdown）。</summary>
internal sealed class ScraplingCrawlResultWindow:Window
{
    public ScraplingCrawlResultWindow(Window? owner,ScraplingCrawlService.CrawlResult result,string? savedPath)
    {
        Title=LocalizationService.T("Scrapling 抓取结果","Scrapling crawl result");
        Width=640;Height=560;
        WindowStartupLocation=owner is null?WindowStartupLocation.CenterScreen:WindowStartupLocation.CenterOwner;
        if(owner is not null){Owner=owner;Topmost=owner.Topmost;}
        ShowInTaskbar=false;
        Background=new SolidColorBrush(Color.FromRgb(246,248,252));
        var text=new TextBox
        {
            Text=result.Text,
            IsReadOnly=true,
            TextWrapping=TextWrapping.Wrap,
            VerticalScrollBarVisibility=ScrollBarVisibility.Auto,
            FontSize=13,
            Padding=new Thickness(10),
            Background=new SolidColorBrush(Color.FromRgb(255,255,255)),
        };
        var header=new TextBlock
        {
            Text=$"{result.Title}\n{result.Url}",
            TextWrapping=TextWrapping.Wrap,
            FontWeight=FontWeights.SemiBold,
            FontSize=13,
            Foreground=new SolidColorBrush(Color.FromRgb(38,52,74)),
            Margin=new Thickness(0,0,0,4),
        };
        var saved=savedPath is null
            ?new TextBlock{Text=LocalizationService.T("（正文未能自动保存，可点“复制全文”）","(Auto-save failed; use Copy all)"),FontSize=11,Foreground=new SolidColorBrush(Color.FromRgb(160,170,186)),Margin=new Thickness(0,0,0,8),TextWrapping=TextWrapping.Wrap}
            :new TextBlock{Text=LocalizationService.T($"已保存到：{savedPath}",$"Saved to: {savedPath}"),FontSize=11,Foreground=new SolidColorBrush(Color.FromRgb(101,116,138)),Margin=new Thickness(0,0,0,8),TextWrapping=TextWrapping.Wrap};
        var copyAll=new Button{Content=LocalizationService.T("复制全文","Copy all"),Padding=new Thickness(14,4,14,4)};
        copyAll.SetResourceReference(StyleProperty,"PrimaryButton");
        copyAll.Click+=(_,_)=>ClipboardService.TrySetText($"{result.Title}\n{result.Url}\n\n{result.Text}",out _);
        var copyLink=new Button{Content=LocalizationService.T("复制链接","Copy link"),Padding=new Thickness(14,4,14,4)};
        copyLink.SetResourceReference(StyleProperty,"SecondaryButton");
        copyLink.Click+=(_,_)=>ClipboardService.TrySetText(result.Url,out _);
        var buttons=new StackPanel{Orientation=Orientation.Horizontal,HorizontalAlignment=HorizontalAlignment.Right,Margin=new Thickness(0,8,0,8)};
        buttons.Children.Add(copyLink);buttons.Children.Add(copyAll);
        if(savedPath is not null)
        {
            var openFolder=new Button{Content=LocalizationService.T("打开所在文件夹","Open folder"),Padding=new Thickness(14,4,14,4)};
            openFolder.SetResourceReference(StyleProperty,"SecondaryButton");
            openFolder.Click+=(_,_)=>{
                try
                {
                    var psi=new System.Diagnostics.ProcessStartInfo("explorer.exe",$"/select,\"{savedPath}\""){UseShellExecute=true};
                    System.Diagnostics.Process.Start(psi);
                }
                catch(Exception ex){new PrivacyLogger().Info("ScraplingCrawlOpenFolder",ex.GetType().Name);}
            };
            buttons.Children.Insert(0,openFolder);
        }
        var panel=new StackPanel{Margin=new Thickness(18)};
        panel.Children.Add(header);panel.Children.Add(saved);panel.Children.Add(buttons);
        panel.Children.Add(text);
        Content=panel;
        Loaded+=(_,_)=>text.Focus();
    }
}

/// <summary>未安装 Scrapling 时的安装指引窗：说明用途、安装步骤与期望的目录结构。</summary>
internal sealed class ScraplingSetupHintWindow:Window
{
    public ScraplingSetupHintWindow(Window? owner)
    {
        Title=LocalizationService.T("需要安装 Scrapling 爬虫环境","Scrapling crawler required");
        Width=560;SizeToContent=SizeToContent.Height;
        WindowStartupLocation=owner is null?WindowStartupLocation.CenterScreen:WindowStartupLocation.CenterOwner;
        if(owner is not null){Owner=owner;Topmost=owner.Topmost;}
        ShowInTaskbar=false;ResizeMode=ResizeMode.NoResize;
        Background=new SolidColorBrush(Color.FromRgb(246,248,252));
        var steps=LocalizationService.IsEnglish
            ?"Scrapling is an open-source Python crawler (https://github.com/D4Vinci/Scrapling) that fetches pages a normal browser cannot (e.g. WeChat articles).\n\nInstall steps:\n1. Create a folder, e.g. D:\\scrapling_app\n2. Inside it run:\n    python -m venv venv\n    venv\\Scripts\\pip install scrapling\n3. For WeChat articles also run:\n    venv\\Scripts\\scrapling install\n   (this downloads the stealth browser into a browsers\\ subfolder)\n\nMewuAI auto-detects any folder containing venv\\Scripts\\python.exe among: D:\\scrapling_app, D:\\scrapling, C:\\scrapling_app, Desktop\\爬虫\\scrapling, D:\\爬虫\\scrapling — or set the path in settings (ScraplingPath).\nUntil then, \"Open link\" and \"Copy link\" keep working."
            :"Scrapling 是开源 Python 爬虫（github.com/D4Vinci/Scrapling），可抓取普通浏览器打不开的页面（如微信公众号文章）。\n\n安装步骤：\n1. 新建文件夹，例如 D:\\scrapling_app\n2. 在文件夹内执行：\n    python -m venv venv\n    venv\\Scripts\\pip install scrapling\n3. 抓微信公众号文章还需执行：\n    venv\\Scripts\\scrapling install\n   （下载隐身浏览器到 browsers\\ 子目录）\n\nMewuAI 会自动探测下列任一含 venv\\Scripts\\python.exe 的目录：D:\\scrapling_app、D:\\scrapling（爬虫）、C:\\scrapling_app、桌面\\爬虫\\scrapling、D:\\爬虫\\scrapling。也可在设置中显式指定路径（ScraplingPath）。未安装前“打开链接/复制链接”不受影响。";
        var body=new TextBlock{Text=steps,TextWrapping=TextWrapping.Wrap,FontSize=12.5,Foreground=new SolidColorBrush(Color.FromRgb(38,52,74)),LineHeight=20};
        var close=new Button{Content=LocalizationService.T("知道了","Got it"),Padding=new Thickness(18,5,18,5),HorizontalAlignment=HorizontalAlignment.Right,Margin=new Thickness(0,14,0,0),IsCancel=true};
        close.SetResourceReference(StyleProperty,"PrimaryButton");
        var panel=new StackPanel{Margin=new Thickness(18)};
        panel.Children.Add(new TextBlock{Text=LocalizationService.T("未找到本机 Scrapling 环境","No local Scrapling installation found"),FontWeight=FontWeights.SemiBold,FontSize=15,Foreground=new SolidColorBrush(Color.FromRgb(38,52,74)),Margin=new Thickness(0,0,0,10)});
        panel.Children.Add(body);
        panel.Children.Add(close);
        Content=panel;
        Loaded+=(_,_)=>close.Focus();
    }
}

/// <summary>识别屏幕邮箱后弹出的撰写窗口；发送前均需用户在确认对话框中放行。
/// 发送通道由调用方注入（QQ 邮箱 MCP 两阶段确认 / 网易邮箱 SMTP）。</summary>
internal sealed class MailComposeWindow:Window
{
    private readonly Window _owner;
    private readonly string _address;
    private readonly TextBox _subject=new(){Padding=new Thickness(8,5,8,5)};
    private readonly TextBox _body=new(){AcceptsReturn=true,TextWrapping=TextWrapping.Wrap,VerticalScrollBarVisibility=ScrollBarVisibility.Auto,Height=150,Padding=new Thickness(8,5,8,5)};
    private readonly Button _send;
    private readonly TextBlock _status=new(){TextWrapping=TextWrapping.Wrap,Foreground=new SolidColorBrush(Color.FromRgb(119,131,152)),FontSize=12};
    private readonly Func<string,string,CancellationToken,Task<(bool Success,string Message)>> _sendAsync;
    private readonly CancellationTokenSource _cancellation=new();

    /// <summary>发送成功并关闭窗口后回调（用于收起识屏浮层）。</summary>
    internal Action? OnSent{get;set;}

    internal MailComposeWindow(Window owner,string address,string title,Func<string,string,CancellationToken,Task<(bool Success,string Message)>> sendAsync)
    {
        _owner=owner;_address=address;_sendAsync=sendAsync;
        Title=title;
        Width=480;SizeToContent=SizeToContent.Height;
        WindowStartupLocation=WindowStartupLocation.CenterOwner;
        Owner=owner;Topmost=owner.Topmost;
        ShowInTaskbar=false;ResizeMode=ResizeMode.NoResize;
        Background=new SolidColorBrush(Color.FromRgb(246,248,252));
        _subject.Text=LocalizationService.T("你好","Hello");
        _send=new Button{Content=LocalizationService.T("发送","Send"),Padding=new Thickness(18,5,18,5),IsDefault=true};
        _send.SetResourceReference(StyleProperty,"PrimaryButton");
        var cancel=new Button{Content=LocalizationService.T("取消","Cancel"),Padding=new Thickness(18,5,18,5),IsCancel=true};
        cancel.SetResourceReference(StyleProperty,"SecondaryButton");
        var panel=new StackPanel{Margin=new Thickness(18)};
        panel.Children.Add(Label(LocalizationService.T("收件人","To")));
        var to=new TextBox{Text=address,IsReadOnly=true,Padding=new Thickness(8,5,8,5),Background=new SolidColorBrush(Color.FromRgb(240,243,249))};
        panel.Children.Add(to);
        panel.Children.Add(Label(LocalizationService.T("主题","Subject")));
        panel.Children.Add(_subject);
        panel.Children.Add(Label(LocalizationService.T("正文","Body")));
        panel.Children.Add(_body);
        var buttons=new StackPanel{Orientation=Orientation.Horizontal,HorizontalAlignment=HorizontalAlignment.Right,Margin=new Thickness(0,14,0,8)};
        buttons.Children.Add(cancel);buttons.Children.Add(_send);
        panel.Children.Add(buttons);
        panel.Children.Add(_status);
        Content=panel;
        _send.Click+=async(_,_)=>await SendAsync();
        Loaded+=(_,_)=>{_subject.Focus();_subject.SelectAll();};
        Closed+=(_,_)=>_cancellation.Cancel();
    }

    private static TextBlock Label(string text)=>new(){Text=text,Margin=new Thickness(0,12,0,4),Foreground=new SolidColorBrush(Color.FromRgb(71,84,108)),FontSize=12};

    private async Task SendAsync()
    {
        var subject=_subject.Text.Trim();
        var body=_body.Text.Trim();
        if(subject.Length==0||body.Length==0)
        {
            _status.Text=LocalizationService.T("主题和正文不能为空。","Subject and body are required.");
            return;
        }
        _send.IsEnabled=false;
        _status.Text=LocalizationService.T("正在发送…","Sending…");
        try
        {
            var (success,message)=await _sendAsync(subject,body,_cancellation.Token).ConfigureAwait(true);
            _status.Text=message;
            if(success)
            {
                await Task.Delay(1000);
                OnSent?.Invoke();
                Close();
            }
            else _send.IsEnabled=true;
        }
        catch(OperationCanceledException)
        {
            _status.Text=LocalizationService.T("已取消发送。","Send canceled.");
            _send.IsEnabled=true;
        }
        catch(Exception ex)
        {
            _status.Text=ex.Message;
            _send.IsEnabled=true;
        }
    }
}
