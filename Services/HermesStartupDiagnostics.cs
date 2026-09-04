namespace mewu_ai_Assistant.Services;

// Retain only allowlisted error categories, never raw stderr, paths, prompts,
// credentials or environment values. Both output channels may report failures.
internal sealed class HermesStartupDiagnostics
{
    private int _categories;

    internal void Observe(string? line)
    {
        if (string.IsNullOrEmpty(line) || line.Length > 4096) return;
        var category = 0;
        if (line.Contains("ModuleNotFoundError", StringComparison.Ordinal) ||
            line.Contains("ImportError", StringComparison.Ordinal) ||
            line.Contains("Web UI dependencies not installed", StringComparison.Ordinal)) category |= 1;
        if (line.Contains("Fatal error in launcher", StringComparison.OrdinalIgnoreCase) ||
            line.Contains("Unable to create process", StringComparison.OrdinalIgnoreCase)) category |= 2;
        if (line.Contains("PermissionError", StringComparison.Ordinal) ||
            line.Contains("WinError 5", StringComparison.Ordinal)) category |= 4;
        if (line.Contains("UnicodeEncodeError", StringComparison.Ordinal) ||
            line.Contains("UnicodeDecodeError", StringComparison.Ordinal)) category |= 8;
        if (line.Contains("invalid choice", StringComparison.Ordinal) ||
            line.Contains("unrecognized arguments", StringComparison.Ordinal)) category |= 16;
        if (line.Contains("address already in use", StringComparison.OrdinalIgnoreCase) ||
            line.Contains("WinError 10048", StringComparison.Ordinal)) category |= 32;
        if (line.Contains("ValidationError", StringComparison.Ordinal) ||
            line.Contains("yaml.scanner.ScannerError", StringComparison.Ordinal) ||
            line.Contains("yaml.parser.ParserError", StringComparison.Ordinal)) category |= 64;
        Interlocked.Or(ref _categories, category);
    }

    internal string Describe(int exitCode)
    {
        var categories = Volatile.Read(ref _categories);
        var hints = new List<string>();
        void Add(int flag, string zh, string en) { if ((categories & flag) != 0) hints.Add(LocalizationService.T(zh, en)); }
        Add(1, "Python 模块或依赖加载失败", "Python module/dependency import failed");
        Add(2, "Hermes 启动器无法创建 Python 进程", "Hermes launcher could not create its Python process");
        Add(4, "启动时访问被拒绝", "Access denied during startup");
        Add(8, "启动输出编码不兼容", "Startup text encoding failed");
        Add(16, "Hermes 版本不支持启动参数", "Hermes does not support the startup arguments");
        Add(32, "监听端口被占用", "Listening port is already in use");
        Add(64, "Hermes 配置解析或校验失败", "Hermes configuration parsing or validation failed");
        var detail = hints.Count == 0
            ? LocalizationService.T("未收到可安全显示的具体错误；不能仅凭退出码判定为安装损坏。", "No safely reportable error category was received; the exit code alone does not prove a broken installation.")
            : string.Join("；", hints);
        return LocalizationService.T($"Hermes 后台启动失败（代码 {exitCode}）。{detail}", $"Hermes backend startup failed (exit code {exitCode}). {detail}");
    }
}
