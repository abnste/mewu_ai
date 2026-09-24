// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.IO;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;

/// <summary>
/// 调用本机 Scrapling（D:\scrapling_app 等安装目录，venv 隔离环境）无头抓取网页内容。
/// 给定 URL 即可抓取诸如微信公众号文章等动态/反爬页面：先轻量 Fetcher，疑似被拦截
/// （知乎 403 等）自动升级 StealthyFetcher（隐身浏览器）。微信 /s/ 链接返回错误页时
/// 会自动尝试 OCR 易混字符（I/l/1、O/0）变体并纠正链接。结果返回标题+正文文本。
/// 微信公众号文章以 #js_content 元素抽取（仅取正文容器，不混入页面 UI）。
/// </summary>
internal static class ScraplingCrawlService
{
    /// <summary>常见安装位置（按顺序探测），亦可在设置中显式指定 ScraplingPath。</summary>
    private static readonly string[] KnownPaths=
    [
        @"D:\scrapling（爬虫）",
        @"D:\scrapling_app",
        @"D:\scrapling",
        @"C:\scrapling_app",
    ];

    /// <summary>找到可用的 Scrapling 安装目录（含 venv\Scripts\python.exe），找不到返回 null。
    /// 探测顺序：
    /// 1. 用户在设置中显式指定的路径（settings.ScraplingPath）
    /// 2. 桌面"爬虫"目录下的所有快捷方式（.lnk）：解析 TargetPath 取安装目录
    /// 3. 桌面"爬虫"目录的子目录（递归 2 层）以及常见位置
    /// 4. 找到第一个含 venv/Scripts/python.exe 的目录。</summary>
    internal static string? Locate(AppSettings settings)
    {
        var seen=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var candidates=new List<string>();
        void Add(string? dir)
        {
            if(string.IsNullOrWhiteSpace(dir))return;
            dir=NormalizeInstallPath(dir);
            if(seen.Add(dir))candidates.Add(dir);
        }

        Add(settings.ScraplingPath);
        foreach(var known in KnownPaths)Add(known);

        var desktop=Environment.GetFolderPath(Environment.SpecialFolder.Desktop);
        var crawlDir=Path.Combine(desktop,"爬虫");
        // 1. 桌面"爬虫"目录里的所有 .lnk 快捷方式：解析 TargetPath，取其所在盘符/祖先目录。
        //    用户在桌面建 scrapling 启动快捷方式时常用此布局，TargetPath 通常直接
        //    指向 scrapling 安装目录里的某个 .exe 或 .bat。
        if(Directory.Exists(crawlDir))
        {
            try
            {
                foreach(var lnk in Directory.EnumerateFiles(crawlDir,"*.lnk",SearchOption.TopDirectoryOnly))
                {
                    var target=ShellLinkResolver.ResolveTarget(lnk);
                    if(string.IsNullOrWhiteSpace(target))continue;
                    Add(target);
                    var dir=Path.GetDirectoryName(target);
                    Add(dir);
                    // .lnk 指向 python.exe 时，scrapling 安装根目录在 venv/Scripts 的上两级。
                    if(target.EndsWith("python.exe",StringComparison.OrdinalIgnoreCase)&&dir is not null)
                        Add(Path.GetDirectoryName(Path.GetDirectoryName(dir)));
                }
            }
            catch{ /* 探测失败继续 */ }
            // 2. 桌面"爬虫"目录的子目录（递归 2 层）：scrapling 可能直接放在子目录里
            //    而非固定的 scrapling 子目录名。
            foreach(var sub in EnumerateDirectoriesSafe(crawlDir,2))
            {
                Add(sub);
                Add(Path.Combine(sub,"scrapling"));
            }
        }
        Add(Path.Combine(desktop,"爬虫","scrapling"));
        Add(Path.Combine(desktop,"scrapling"));
        Add(@"D:\Downloads\爬虫\scrapling");
        Add(@"D:\爬虫\scrapling");

        foreach(var dir in candidates)
        {
            try
            {
                var python=Path.Combine(dir,"venv","Scripts","python.exe");
                new PrivacyLogger().Info("ScraplingLocate",$"candidate={dir} python={File.Exists(python)}");
                if(File.Exists(python))return dir;
            }
            catch{ /* 探测失败继续 */ }
        }
        return null;
    }

    private static string NormalizeInstallPath(string path)
    {
        var value=path.Trim().Trim('"');
        try
        {
            if(File.Exists(value))
            {
                var name=Path.GetFileName(value);
                if(name.EndsWith(".lnk",StringComparison.OrdinalIgnoreCase))
                {
                    var target=ShellLinkResolver.ResolveTarget(value);
                    if(!string.IsNullOrWhiteSpace(target))value=target!;
                }
                if(File.Exists(value))value=Path.GetDirectoryName(value)??value;
            }
            if(value.EndsWith(".bat",StringComparison.OrdinalIgnoreCase)||value.EndsWith(".cmd",StringComparison.OrdinalIgnoreCase)||value.EndsWith(".py",StringComparison.OrdinalIgnoreCase))
                value=Path.GetDirectoryName(value)??value;
            var pythonMarker=Path.Combine("venv","Scripts","python.exe");
            if(value.EndsWith(pythonMarker,StringComparison.OrdinalIgnoreCase))
                value=value[..^pythonMarker.Length].TrimEnd('\\','/');
        }
        catch{}
        return value;
    }

    private static IEnumerable<string> EnumerateDirectoriesSafe(string root,int maxDepth)
    {
        if(!Directory.Exists(root))yield break;
        var stack=new Stack<(string path,int depth)>();
        stack.Push((root,0));
        while(stack.Count>0)
        {
            var (path,depth)=stack.Pop();
            IEnumerable<string> subs;
            try{subs=Directory.EnumerateDirectories(path);}
            catch{continue;}
            foreach(var sub in subs)
            {
                yield return sub;
                if(depth+1<maxDepth)stack.Push((sub,depth+1));
            }
        }
    }

    internal sealed record CrawlResult(string Url,string Title,string Text,string? CorrectedFrom=null,string? Engine=null);

    /// <summary>抓取 URL 内容。调用方负责在后台线程执行（浏览器抓取可能需要数十秒）。</summary>
    internal static async Task<CrawlResult> CrawlAsync(string url,string? scraplingDir,CancellationToken cancellationToken)
    {
        if(string.IsNullOrWhiteSpace(scraplingDir))
            throw new InvalidOperationException(LocalizationService.T("未找到 Scrapling 环境。","Scrapling environment was not found."));
        new PrivacyLogger().Info("ScraplingStage",$"process_start dir={scraplingDir}");
        var python=Path.Combine(scraplingDir,"venv","Scripts","python.exe");
        if(!File.Exists(python))
            throw new InvalidOperationException(LocalizationService.T("Scrapling 环境不完整：缺少 venv\\Scripts\\python.exe。","Scrapling environment incomplete: venv\\Scripts\\python.exe is missing."));
        var script=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","scrapling_fetch.py");
        Directory.CreateDirectory(Path.GetDirectoryName(script)!);
        await File.WriteAllTextAsync(script,FetchScript,Encoding.UTF8,cancellationToken).ConfigureAwait(false);

        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromMinutes(3));
        var psi=new ProcessStartInfo(python)
        {
            WorkingDirectory=scraplingDir,
            UseShellExecute=false,
            CreateNoWindow=true,
            RedirectStandardOutput=true,
            RedirectStandardError=true,
            StandardOutputEncoding=Encoding.UTF8,
            StandardErrorEncoding=Encoding.UTF8,
        };
        // StealthyFetcher 需要项目自带的 playwright 浏览器目录。
        var browsers=Path.Combine(scraplingDir,"browsers");
        if(Directory.Exists(browsers))psi.EnvironmentVariables["PLAYWRIGHT_BROWSERS_PATH"]=browsers;
        // 保留用户的有效代理（访问 google.com 等站点需要），但剔除格式损坏的
        // 代理变量（如 http://http://…，会让 curl_cffi 直接 ProxyError）。
        SanitizeProxyEnv(psi.EnvironmentVariables);
        psi.ArgumentList.Add(script);
        psi.ArgumentList.Add(url);
        using var process=Process.Start(psi)
            ??throw new InvalidOperationException(LocalizationService.T("无法启动 Scrapling Python 进程。","Failed to start the Scrapling Python process."));
        var stdoutTask=process.StandardOutput.ReadToEndAsync(timeout.Token);
        var stderrTask=process.StandardError.ReadToEndAsync(timeout.Token);
        try{await process.WaitForExitAsync(timeout.Token).ConfigureAwait(false);}
        finally{if(!process.HasExited){try{process.Kill(entireProcessTree:true);}catch{} }}
        var stdout=await stdoutTask.ConfigureAwait(false);
        var stderr=await stderrTask.ConfigureAwait(false);
        if(!string.IsNullOrWhiteSpace(stderr))
            new PrivacyLogger().Info("ScraplingStderr",Truncate(stderr,400));
        // stdout 最后一行是 JSON 结果（前面可能有日志输出）。
        var jsonLine=stdout.Split('\n').Select(line=>line.Trim()).LastOrDefault(line=>
        {
            if(line.Length<2||line[0]!='{'||line[^1]!='}')return false;
            try{using var probe=JsonDocument.Parse(line);return probe.RootElement.ValueKind==JsonValueKind.Object&&probe.RootElement.TryGetProperty("url",out _);}
            catch(JsonException){return false;}
        });
        if(jsonLine is null)
            throw new InvalidOperationException(string.Format(System.Globalization.CultureInfo.CurrentCulture,LocalizationService.T("Scrapling 未返回 JSON 结果：{0}","Scrapling returned no JSON result: {0}"),Truncate(stderr+stdout,500)));
        JsonDocument doc;
        try{doc=JsonDocument.Parse(jsonLine.Trim());}
        catch(JsonException ex){throw new InvalidOperationException(LocalizationService.T("Scrapling 返回了无效 JSON。","Scrapling returned invalid JSON."),ex);}
        using(doc)
        {
        var root=doc.RootElement;
        if(root.TryGetProperty("error",out var error))
            throw new InvalidOperationException(TranslateCrawlError(error.GetString()));
        var resultUrl=root.TryGetProperty("url",out var urlNode)?urlNode.GetString():url;
        var title=root.TryGetProperty("title",out var titleNode)?titleNode.GetString():string.Empty;
        var text=root.TryGetProperty("text",out var textNode)?textNode.GetString():string.Empty;
        var correctedFrom=root.TryGetProperty("corrected_from",out var cfNode)?cfNode.GetString():null;
        var textLength=text?.Length??0;
        var hasParameterError=text?.Contains("Parameter error",StringComparison.OrdinalIgnoreCase)==true;
        new PrivacyLogger().Info("ScraplingResult",$"urlHost={new Uri(url).Host};length={textLength};parameterError={hasParameterError};titleLength={title?.Length??0};corrected={correctedFrom is not null}");
        return new CrawlResult(resultUrl??url,title??string.Empty,text??string.Empty,correctedFrom);
        }
    }

    /// <summary>把抓取脚本返回的结构化错误码翻译成清晰的用户提示。</summary>
    private static string TranslateCrawlError(string? code)
    {
        return (code??string.Empty) switch
        {
            "weixin_error_page"=>LocalizationService.T(
                "微信返回了错误页（Parameter error）：文章链接无效，或圈选识别时把 I/l/1、O/0 等易混字符看错了。已自动尝试常见混淆组合仍未成功，请核对链接后重试。",
                "WeChat returned an error page (Parameter error): the article link is invalid, or characters like I/l/1 and O/0 were misread during on-screen recognition. Common confusions were retried automatically without success; please verify the link and try again."),
            "network_unreachable"=>LocalizationService.T(
                "无法连接到目标网站：可能需要代理（如 google.com）、系统代理配置有误或站点不可达。请先在浏览器中确认该链接能打开。",
                "Could not connect to the target site: a proxy may be required (e.g. google.com), the system proxy may be misconfigured, or the site is unreachable. Verify the link opens in a browser first."),
            ""=>LocalizationService.T("Scrapling 抓取失败。","Scrapling crawl failed."),
            _=>code!,
        };
    }

    /// <summary>剔除格式损坏的代理环境变量（值形如 http://http://…、无主机名等），
    /// 保留合法代理（用户可能依赖本地代理访问外网）。curl_cffi 遇到坏代理会直接
    /// ProxyError，playwright 也会受影响。</summary>
    private static void SanitizeProxyEnv(System.Collections.Specialized.StringDictionary env)
    {
        foreach(var name in new[]{"HTTP_PROXY","HTTPS_PROXY","http_proxy","https_proxy","ALL_PROXY","all_proxy"})
        {
            try
            {
                if(env[name] is null)continue;
                var value=env[name];
                if(string.IsNullOrWhiteSpace(value)||!Uri.TryCreate(value.Trim(),UriKind.Absolute,out var parsed)
                    ||(parsed.Scheme!="http"&&parsed.Scheme!="https"&&parsed.Scheme!="socks5"&&parsed.Scheme!="socks5h")
                    ||string.IsNullOrWhiteSpace(parsed.Host))
                {
                    env.Remove(name);
                }
            }
            catch{ /* 单项失败不影响其他 */ }
        }
    }

    private static string Truncate(string value,int max)=>value.Length<=max?value:value[..max];

    /// <summary>安装结果：成功时 InstallDir 为新建的 venv 根目录（含 venv\\Scripts\\python.exe）；
    /// 失败时 Message 是用户可读的错误描述，详细 stderr 已写到 MewuAI 日志。</summary>
    internal sealed record InstallResult(bool Success,string InstallDir,string Message);

    /// <summary>手动安装步骤（剪贴板用）：含检查 Python + 离线 / 在线两种安装方式。
    /// 当一键安装失败或用户希望自己装时复制到剪贴板。</summary>
    internal static string ManualInstallSteps()
    {
        return LocalizationService.IsEnglish
            ?"# Install Scrapling manually\n\n" +
             "# 1) Find a Python 3.10+ interpreter\npy -3.13 -V  # or python / python3\n\n" +
             "# 2) Create a venv and install\npy -3.13 -m venv %LOCALAPPDATA%\\MewuAI\\scrapling\n%LOCALAPPDATA%\\MewuAI\\scrapling\\venv\\Scripts\\pip install --upgrade pip\n%LOCALAPPDATA%\\MewuAI\\scrapling\\venv\\Scripts\\pip install scrapling\n\n" +
             "# 3) Download the stealth browser (Chromium for playwright)\n%LOCALAPPDATA%\\MewuAI\\scrapling\\venv\\Scripts\\scrapling install\n\n" +
             "# Then click \"Scrapling 爬取\" again. MewuAI auto-detects the install via Locate()."
            :"# 手动安装 Scrapling\n\n" +
             "# 1) 找到 Python 3.10+ 解释器（任选其一）\npy -3.13 -V   # 或 python / python3\n\n" +
             "# 2) 建虚拟环境并安装 Scrapling\npy -3.13 -m venv %LOCALAPPDATA%\\MewuAI\\scrapling\n%LOCALAPPDATA%\\MewuAI\\scrapling\\venv\\Scripts\\pip install --upgrade pip\n%LOCALAPPDATA%\\MewuAI\\scrapling\\venv\\Scripts\\pip install scrapling\n\n" +
             "# 3) 下载隐身浏览器（playwright 内置 Chromium）\n%LOCALAPPDATA%\\MewuAI\\scrapling\\venv\\Scripts\\scrapling install\n\n" +
             "# 完成后再次点击\"Scrapling 爬取\"。mewuAI 通过 Locate() 自动发现安装。";
    }

    /// <summary>探测 PATH 里可用的 Python 3.10+ 解释器。优先级：py 启动器（先 -3.13、
    /// -3.12、-3.10，回退默认）→ python3 → python。返回 (解释器路径, 版本字符串)，
    /// 没找到返回 (null, null)。</summary>
    private static (string? Path,string? Version) DetectPython()
    {
        var candidates=new List<(string machine, string[] args)>
        {
            ("py", new[]{"-3.13"}),
            ("py", new[]{"-3.12"}),
            ("py", new[]{"-3.11"}),
            ("py", new[]{"-3.10"}),
            ("py", Array.Empty<string>()),
            ("python", Array.Empty<string>()),
            ("python3", Array.Empty<string>()),
        };
        foreach(var (machine, args) in candidates)
        {
            try
            {
                var exe=LocateExecutable(machine);
                if(string.IsNullOrEmpty(exe))continue;
                var psi=new ProcessStartInfo(exe,args.Length>0?string.Join(' ',args):"--version"){UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true};
                using var p=Process.Start(psi);
                if(p is null)continue;
                var stdout=p.StandardOutput.ReadToEnd().Trim();
                var stderr=p.StandardError.ReadToEnd().Trim();
                p.WaitForExit(5000);
                if(p.ExitCode!=0)continue;
                var combined=stdout.Length>0?stdout:stderr;
                if(!combined.StartsWith("Python ",StringComparison.OrdinalIgnoreCase))continue;
                var version=combined.Substring("Python ".Length).Trim();
                // 只用 3.10+（Scrapling 需要）。
                var dot=version.IndexOf('.');
                if(dot<=0)continue;
                if(!int.TryParse(version.AsSpan(0,dot),out var major))continue;
                if(major<3||major>3&&version.Length<=dot+1)continue;
                if(major==3)
                {
                    var secondDot=version.IndexOf('.',dot+1);
                    if(secondDot<0)continue;
                    if(!int.TryParse(version.AsSpan(dot+1,secondDot-dot-1),out var minor))continue;
                    if(minor<10)continue;
                }
                return (exe, version);
            }
            catch{ /* 探测失败继续 */ }
        }
        return (null,null);
    }

    /// <summary>把一个可执行文件名解析为完整路径（环境变量 PATH + 标准 Windows 安装目录）。
    /// 不调用 Process.Start 直接给"py"，因为 PyLauncher 不在 System32 也不在 PATH，
    /// 只在 WindowsApps 里。</summary>
    private static string? LocateExecutable(string name)
    {
        if(Path.IsPathRooted(name)&&File.Exists(name))return name;
        var pathEnv=Environment.GetEnvironmentVariable("PATH")??string.Empty;
        var sep=Path.PathSeparator;
        foreach(var dir in pathEnv.Split(sep,StringSplitOptions.RemoveEmptyEntries))
        {
            try
            {
                var candidate=Path.Combine(dir.Trim('"'),name+".exe");
                if(File.Exists(candidate))return candidate;
                var candidate2=Path.Combine(dir.Trim('"'),name);
                if(File.Exists(candidate2))return candidate2;
            }
            catch{}
        }
        // py 启动器（Windows Launcher）在 %LOCALAPPDATA%\Microsoft\WindowsApps 与
        // C:\Windows 两个位置。
        if(string.Equals(name,"py",StringComparison.OrdinalIgnoreCase))
        {
            var wellKnown=new[]{
                Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),@"Microsoft\WindowsApps","py.exe"),
                @"C:\Windows\py.exe",
            };
            foreach(var candidate in wellKnown)if(File.Exists(candidate))return candidate;
        }
        return null;
    }

    /// <summary>默认安装目录：%LOCALAPPDATA%\MewuAI\scrapling（含 venv\Scripts\python.exe）。
    /// 优先使用设置里显式指定的路径，否则用默认。</summary>
    internal static string DefaultInstallDir()
    {
        var local=Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        return Path.Combine(local,"MewuAI","scrapling");
    }

    /// <summary>在一键安装流程里实时上报步骤（UI 进度条 / PromptStatus 文本）。
    /// 调用方要负责把字符串转发到 UI 线程。</summary>
    internal static async Task<InstallResult> EnsureInstalledAsync(IProgress<string>? progress,CancellationToken cancellationToken)
    {
        progress?.Report(LocalizationService.T("正在探测 Python 解释器…","Detecting Python interpreter…"));
        var (pythonExe, version)=DetectPython();
        if(pythonExe is null)
        {
            new PrivacyLogger().Info("ScraplingInstall","no_python_found");
            return new InstallResult(false,string.Empty,LocalizationService.T(
                "未找到 Python 3.10+ 解释器。请先到 https://www.python.org/downloads/ 安装 Python 3.10 或更高版本，然后重试。",
                "No Python 3.10+ interpreter found. Install Python 3.10 or newer from https://www.python.org/downloads/ and try again."));
        }
        progress?.Report(string.Format(System.Globalization.CultureInfo.CurrentCulture,
            LocalizationService.T("找到 Python {0}：{1}","Found Python {0}: {1}"),version!,pythonExe!));

        var installDir=DefaultInstallDir();
        var python=Path.Combine(installDir,"venv","Scripts","python.exe");
        if(File.Exists(python))
        {
            progress?.Report(LocalizationService.T("venv 已存在，跳过创建。","venv already exists; skipping creation."));
        }
        else
        {
            progress?.Report(LocalizationService.T("创建 venv…","Creating venv…"));
            Directory.CreateDirectory(installDir);
            var venvArgs=new ProcessStartInfo(pythonExe,$"-m venv \"{Path.Combine(installDir,"venv")}\""){UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,StandardOutputEncoding=Encoding.UTF8,StandardErrorEncoding=Encoding.UTF8};
            ClearBadProxyEnv(venvArgs.EnvironmentVariables);
            using var vp=Process.Start(venvArgs);
            if(vp is null)return new InstallResult(false,installDir,LocalizationService.T("无法启动 Python 进程。","Failed to start the Python process."));
            var vstderr=vp.StandardError.ReadToEndAsync(cancellationToken).GetAwaiter().GetResult();
            try{await vp.WaitForExitAsync(cancellationToken).ConfigureAwait(false);}catch(OperationCanceledException){return new InstallResult(false,installDir,LocalizationService.T("安装被取消。","Failed to install: canceled."));}
            new PrivacyLogger().Info("ScraplingInstall","venv:"+Truncate(vstderr,400));
            if(vp.ExitCode!=0||!File.Exists(python))
                return new InstallResult(false,installDir,string.Format(System.Globalization.CultureInfo.CurrentCulture,
                    LocalizationService.T("创建 venv 失败（退出码 {0}）。","Failed to create venv (exit code {0})."),vp.ExitCode));
        }

        progress?.Report(LocalizationService.T("升级 pip…","Upgrading pip…"));
        await RunStep(python,new[]{"-m","pip","install","--upgrade","pip"},installDir,progress,cancellationToken).ConfigureAwait(false);
        // pip upgrade 偶尔因为网络失败但不是致命（Scrapling 自带较新的 pip 约束）。

        progress?.Report(LocalizationService.T("安装 scrapling 包（约 30 MB，可能耗时 1-2 分钟）…","Installing scrapling package (~30 MB, may take 1-2 minutes)…"));
        var pkgResult=await RunStep(python,new[]{"-m","pip","install","scrapling"},installDir,progress,cancellationToken).ConfigureAwait(false);
        var pkgExit=pkgResult.exit;
        var pkgStderr=pkgResult.stderr;
        new PrivacyLogger().Info("ScraplingInstall","pip:"+Truncate(pkgStderr,600));
        if(pkgExit!=0)
        {
            return new InstallResult(false,installDir,string.Format(System.Globalization.CultureInfo.CurrentCulture,
                LocalizationService.T("pip install scrapling 失败（退出码 {0}），可能是网络问题。详见 MewuAI 日志。","pip install scrapling failed (exit code {0}), possibly a network issue. See MewuAI log for details."),
                pkgExit));
        }

        progress?.Report(LocalizationService.T("下载隐身浏览器（playwright Chromium，可能 100-200 MB）…","Downloading stealth browser (playwright Chromium, ~100-200 MB)…"));
        var scraplingExe=Path.Combine(installDir,"venv","Scripts","scrapling.exe");
        if(!File.Exists(scraplingExe))
            scraplingExe=python; // fallback：python -m scrapling
        var browserResult=await RunStep(scraplingExe,new[]{"install"},installDir,progress,cancellationToken).ConfigureAwait(false);
        var browserExit=browserResult.exit;
        var browserStderr=browserResult.stderr;
        new PrivacyLogger().Info("ScraplingInstall","scrapling_install:"+Truncate(browserStderr,600));
        if(browserExit!=0)
        {
            // scrapling install 失败不致命——没隐身浏览器也能用普通 Fetcher。
            progress?.Report(LocalizationService.T("⚠️ 隐身浏览器下载失败（仍可出抓普通网页，但微信文章可能抓不到正文）。","⚠️ Stealth browser download failed (still works for regular pages, but WeChat may fall back)."));
        }

        progress?.Report(string.Format(System.Globalization.CultureInfo.CurrentCulture,
            LocalizationService.T("安装完成：{0}","Installation finished: {0}"),installDir));
        return new InstallResult(true,installDir,string.Empty);
    }

    private static async Task<(int exit,string stderr)> RunStep(string exe,string[] args,string workingDir,IProgress<string>? progress,CancellationToken cancellationToken)
    {
        var psi=new ProcessStartInfo(exe){UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,WorkingDirectory=workingDir,StandardOutputEncoding=Encoding.UTF8,StandardErrorEncoding=Encoding.UTF8};
        foreach(var a in args)psi.ArgumentList.Add(a);
        ClearBadProxyEnv(psi.EnvironmentVariables);
        using var p=Process.Start(psi);
        if(p is null)return (-1,"");
        var stderrTask=p.StandardError.ReadToEndAsync(cancellationToken);
        var stdoutTask=p.StandardOutput.ReadToEndAsync(cancellationToken);
        try{await p.WaitForExitAsync(cancellationToken).ConfigureAwait(false);}catch(OperationCanceledException){try{p.Kill(entireProcessTree:true);}catch{} return (-1,"canceled");}
        var stderr=await stderrTask.ConfigureAwait(false);
        var stdout=await stdoutTask.ConfigureAwait(false);
        foreach(var line in stdout.Split('\n').TakeLast(3))
        {
            var trimmed=line.Trim();
            if(trimmed.Length>0)progress?.Report(Sanitize(trimmed));
        }
        return (p.ExitCode, stderr);
    }

    private static void ClearBadProxyEnv(System.Collections.Specialized.StringDictionary env)
    {
        var bad=new[]{"HTTP_PROXY","HTTPS_PROXY","http_proxy","https_proxy","ALL_PROXY","all_proxy"};
        foreach(var name in bad)
        {
            try{env.Remove(name);}catch{}
        }
    }

    private static string Sanitize(string line)
    {
        // 去掉 ANSI 控制字符（pip 进度条输出里有）。
        var sb=new StringBuilder(line.Length);
        foreach(var c in line)
        {
            if(c=='\r')continue;
            if(c==0x1b){sb.Append(' ');continue;} // ESC（控制序列前缀）
            sb.Append(c);
        }
        return sb.ToString().Trim();
    }

    /// <summary>嵌入的抓取脚本（v4）。
    /// 实测结论（2026-09-24）：微信对"纯 HTTP 请求"并不封锁——文章 key 有效时
    /// Fetcher（chrome 伪装）直接返回全文（实测 1377 字）；"Parameter error" 错误页
    /// 只出现在 key 无效时。而圈选识别用的本地 OCR 经常把 key 里的大写 I 认成小写 l
    /// （I/l/1、O/0 互混），导致看起来"怎么抓都失败"。脚本因此：
    /// 1) 先用轻量 Fetcher 抓原始 URL；
    /// 2) 若是微信 /s/ 链接且返回错误页 → 有界生成易混字符变体（全局替换 + 逐位替换，
    ///    最多探测 24 个），逐个用 Fetcher 试探，命中即用纠正后的 URL；
    /// 3) 非微信（或仍未命中）且疑似被拦截（如知乎 403 短文案）→ 升级 StealthyFetcher；
    /// 4) 最终仍是错误页时返回结构化错误码（weixin_error_page / network_unreachable），
    ///    由 C# 侧翻译成清晰的用户提示，不再把错误页当正文透传。
    /// 注意 Scrapling 0.4.x 的 Response 提供 css()（列表）而非 css_first()。</summary>
    private const string FetchScript="""
import sys, json, re

url = sys.argv[1].strip()

def dump(obj):
    print(json.dumps(obj, ensure_ascii=False))

def first_css(page, sel):
    try:
        els = page.css(sel)
        return els[0] if els else None
    except Exception:
        return None

def el_text(el):
    if el is None:
        return ""
    try:
        return (el.get_all_text(separator="\n") or "").strip()
    except Exception:
        try:
            return (el.text or "").strip()
        except Exception:
            return ""

def extract(page):
    # 标题：微信 #activity-name > h1 > title。
    title = ""
    for sel in ("#activity-name", "h1", "title"):
        t = first_css(page, sel)
        if t is not None:
            title = el_text(t)
            if title:
                break
    # 正文：按站点选正文容器（微信 #js_content、知乎 .RichContent-inner、
    # 通用 article/main），全部落空才退回整页文本。
    content = ""
    for sel in ("#js_content", ".RichContent-inner", ".QuestionAnswer-content", "article", "main"):
        c = first_css(page, sel)
        if c is not None:
            candidate = el_text(c)
            if len(candidate) > len(content):
                content = candidate
        if content:
            break
    if not content:
        try:
            content = (page.get_all_text(separator="\n") or "").strip()
        except Exception:
            content = ""
    return title, content

def looks_error_page(text):
    if not text:
        return True
    lowered = text.lower()
    if "parameter error" in lowered and len(text) < 1500:
        return True
    if len(text.strip()) < 80:
        return True
    return False

def try_fetch(u, timeout=30):
    try:
        from scrapling.fetchers import Fetcher
        page = Fetcher.get(u, timeout=timeout, impersonate='chrome', stealthy_headers=True)
        if page is None:
            return "", "", "no_page"
        t, x = extract(page)
        return t, x, ""
    except Exception as e:
        return "", "", f"{type(e).__name__}: {e}"

def try_stealthy(u):
    try:
        from scrapling.fetchers import StealthyFetcher
        page = StealthyFetcher.fetch(u, headless=True, block_images=True, disable_resources=True)
        if page is None:
            return "", "", "no_page"
        t, x = extract(page)
        return t, x, ""
    except Exception as e:
        return "", "", f"{type(e).__name__}: {e}"

weixin = "mp.weixin.qq.com" in url
errors = []
title, text = "", ""

title, text, err = try_fetch(url)
if err:
    errors.append(f"fetch: {err}")

# 微信 /s/ 链接且疑似错误页：本地 OCR 常把 key 里的 I/l/1、O/0 认混，
# 有界生成变体并用轻量 Fetcher 探测（有效 key 的纯 HTTP 请求就能拿到正文）。
corrected = None
if weixin and (not text or looks_error_page(text)):
    m = re.match(r"^(https?://mp\.weixin\.qq\.com/s/)([A-Za-z0-9_-]+)(.*)$", url)
    if m:
        prefix, key, suffix = m.group(1), m.group(2), m.group(3)
        subs = {"I": "l1", "l": "I1", "1": "Il", "O": "0o", "0": "Oo", "o": "O0"}
        variants = []
        # OCR 混淆通常是系统性的：先试全局替换（把所有 I 换成 l 等）。
        for src, dsts in subs.items():
            for dst in dsts:
                variants.append(key.replace(src, dst))
        # 再试逐位替换（只错一个字符的情形）。
        for i, ch in enumerate(key):
            if ch in subs and len(variants) < 48:
                for dst in subs[ch]:
                    variants.append(key[:i] + dst + key[i + 1:])
        seen = set([key])
        tried = 0
        for v in variants:
            if v in seen:
                continue
            seen.add(v)
            tried += 1
            if tried > 24:
                break
            t2, x2, e2 = try_fetch(prefix + v + suffix, timeout=15)
            if e2:
                continue
            if x2 and not looks_error_page(x2):
                corrected = prefix + v + suffix
                title, text = t2, x2
                break

# 非微信站点（或微信仍未命中）：疑似被拦截（如知乎 403 短文案）时升级隐身浏览器。
if corrected is None and (not text or looks_error_page(text)):
    t3, x3, e3 = try_stealthy(url)
    if e3:
        errors.append(f"stealthy: {e3}")
    elif x3 and len(x3) > len(text):
        title, text = t3, x3

text = (text or "").strip()
if not text or looks_error_page(text):
    joined = "; ".join(errors)
    lowered = joined.lower()
    if weixin and not corrected:
        # 微信错误页且变体探测全部失败。
        code = "weixin_error_page"
    elif any(marker in lowered for marker in (
        "could not connect", "could not resolve", "timed out", "timeout",
        "connection", "proxy", "tunnel", "ssl")):
        # 网络层失败：需要代理（如 google.com）、代理配置错误或站点不可达。
        code = "network_unreachable"
    else:
        code = joined or "empty_text"
    dump({"url": url, "error": code})
    sys.exit(0)

out = {"url": corrected or url, "title": title, "text": text[:200000]}
if corrected:
    out["corrected_from"] = url
dump(out)
""";
}
