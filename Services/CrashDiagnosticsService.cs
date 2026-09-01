using System.Diagnostics;
using System.Reflection;
using System.Text.Json;

namespace mewu_ai_Assistant.Services;

internal sealed class CrashDiagnosticsService
{
    private const int SchemaVersion=1;
    private static readonly object StaticGate=new();
    private static CrashDiagnosticsService? _current;
    private readonly object _gate=new();
    private readonly string _markerPath;
    private readonly PrivacyLogger _logger;
    private readonly Func<int,bool> _isProcessAlive;
    private readonly Func<DateTimeOffset> _now;
    private CrashSessionMarker? _marker;

    internal CrashDiagnosticsService(
        string? markerPath=null,
        PrivacyLogger? logger=null,
        Func<int,bool>? isProcessAlive=null,
        Func<DateTimeOffset>? now=null)
    {
        _markerPath=markerPath??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Diagnostics","active-session.json");
        _logger=logger??new PrivacyLogger();
        _isProcessAlive=isProcessAlive??IsProcessAlive;
        _now=now??(static()=>DateTimeOffset.UtcNow);
    }

    internal static void InitializePrimary()
    {
        lock(StaticGate)
        {
            _current??=new CrashDiagnosticsService();
            _current.StartSession(Environment.ProcessId);
        }
    }

    internal static void MarkOperation(string operation)
    {
        CrashDiagnosticsService? current;
        lock(StaticGate)current=_current;
        current?.Mark(operation);
    }

    internal static void MarkCleanExit()
    {
        CrashDiagnosticsService? current;
        lock(StaticGate)current=_current;
        current?.CleanExit();
    }

    internal void StartSession(int processId)
    {
        lock(_gate)
        {
            var previous=ReadMarker();
            if(previous is {CleanExit:false}&&previous.ProcessId!=processId&&!_isProcessAlive(previous.ProcessId))
                _logger.Error("PreviousSessionCrash",new InvalidOperationException($"上次会话异常终止；最后阶段：{NormalizeOperation(previous.LastOperation)}；进程 {previous.ProcessId}；最后心跳 {previous.LastUpdatedUtc:O}"));
            var now=_now();
            _marker=new CrashSessionMarker(SchemaVersion,processId,Assembly.GetExecutingAssembly().GetName().Version?.ToString()??"unknown",now,now,"启动",false);
            WriteMarker(_marker);
            _logger.Info("Session","会话已启动");
        }
    }

    internal void Mark(string operation)
    {
        lock(_gate)
        {
            if(_marker is null)return;
            var normalized=NormalizeOperation(operation);
            _marker=_marker with{LastOperation=normalized,LastUpdatedUtc=_now()};
            WriteMarker(_marker);
            _logger.Info("Operation",normalized);
        }
    }

    internal void CleanExit()
    {
        lock(_gate)
        {
            if(_marker is null)return;
            _marker=_marker with{LastOperation="正常退出",LastUpdatedUtc=_now(),CleanExit=true};
            WriteMarker(_marker);
            _logger.Info("Session","会话正常退出");
        }
    }

    private CrashSessionMarker? ReadMarker()
    {
        try
        {
            if(!File.Exists(_markerPath))return null;
            using var stream=new FileStream(_markerPath,FileMode.Open,FileAccess.Read,FileShare.Read);
            var marker=JsonSerializer.Deserialize<CrashSessionMarker>(stream);
            return marker is {SchemaVersion:SchemaVersion,ProcessId:>0}?marker:null;
        }
        catch(Exception ex){_logger.Error("CrashMarkerRead",ex);return null;}
    }

    private void WriteMarker(CrashSessionMarker marker)
    {
        try
        {
            var directory=Path.GetDirectoryName(_markerPath)??throw new InvalidOperationException("崩溃诊断目录无效");
            Directory.CreateDirectory(directory);
            var temporary=Path.Combine(directory,$".{Path.GetFileName(_markerPath)}.{Guid.NewGuid():N}.tmp");
            try
            {
                using(var stream=new FileStream(temporary,FileMode.CreateNew,FileAccess.Write,FileShare.None))
                {
                    JsonSerializer.Serialize(stream,marker);
                    stream.Flush(true);
                }
                File.Move(temporary,_markerPath,true);
            }
            finally{try{if(File.Exists(temporary))File.Delete(temporary);}catch{}}
        }
        catch(Exception ex){_logger.Error("CrashMarkerWrite",ex);}
    }

    private static string NormalizeOperation(string? operation)
    {
        var value=string.IsNullOrWhiteSpace(operation)?"未知阶段":operation.Trim();
        return value.Length<=120?value:value[..120];
    }

    private static bool IsProcessAlive(int processId)
    {
        try{using var process=Process.GetProcessById(processId);return !process.HasExited;}
        catch(ArgumentException){return false;}
        catch(InvalidOperationException){return false;}
    }

    internal sealed record CrashSessionMarker(
        int SchemaVersion,
        int ProcessId,
        string Version,
        DateTimeOffset StartedUtc,
        DateTimeOffset LastUpdatedUtc,
        string LastOperation,
        bool CleanExit);
}
