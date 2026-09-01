using System.Diagnostics;
using System.Security.Cryptography;
using System.Text.Json;
using System.Text.RegularExpressions;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public sealed partial class HermesBackendService : IDisposable, IAsyncDisposable
{
    private static readonly TimeSpan StartupTimeout=TimeSpan.FromSeconds(90);
    private static readonly TimeSpan ShutdownTimeout=TimeSpan.FromSeconds(5);
    private readonly HermesDiscoveryService _discovery;
    private readonly SemaphoreSlim _gate=new(1,1);
    private Process? _process;
    private HermesConnectionInfo? _connection;
    private string? _sessionToken;
    private int _disposed;

    public HermesBackendService(HermesDiscoveryService? discovery=null)=>_discovery=discovery??new HermesDiscoveryService();

    public HermesInstallation? Discover()=>_discovery.Discover();
    public bool IsRunning=>_process is {HasExited:false}&&_connection is not null;

    public async Task<HermesConnectionInfo> EnsureStartedAsync(CancellationToken cancellationToken)
    {
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed)!=0,this);
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            if(_process is {HasExited:false}&&_connection is not null)return _connection;
            await StopOwnedProcessAsync().ConfigureAwait(false);

            var installation=_discovery.Discover()??throw new InvalidOperationException("未检测到可用的本机 Hermes。请先确认 Hermes 已完整安装。");
            var token=CreateSessionToken();
            var ready=new TaskCompletionSource<int>(TaskCreationOptions.RunContinuationsAsynchronously);
            var readyFile=new TempFileService().NewFile(".json");
            var process=new Process{StartInfo=CreateStartInfo(installation,token,readyFile),EnableRaisingEvents=true};
            DataReceivedEventHandler readyHandler=(_,args)=>
            {
                if(TryParseReadyLine(args.Data,out var port))ready.TrySetResult(port);
            };
            process.OutputDataReceived+=readyHandler;
            process.ErrorDataReceived+=readyHandler;
            process.Exited+=(_,_)=>ready.TrySetException(new InvalidOperationException($"Hermes 本地后端在连接前退出（代码 {SafeExitCode(process)}）。"));
            try
            {
                if(!process.Start())throw new InvalidOperationException("Hermes 本地后端未能启动。");
                process.BeginOutputReadLine();
                process.BeginErrorReadLine();
                using var timeout=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
                timeout.CancelAfter(StartupTimeout);
                var readyFilePoll=PollReadyFileAsync(readyFile,ready,timeout.Token);
                int port;
                try{port=await ready.Task.WaitAsync(timeout.Token).ConfigureAwait(false);}
                catch(OperationCanceledException) when(!cancellationToken.IsCancellationRequested)
                {throw new TimeoutException("Hermes 本地后端启动超时，请检查 Hermes 安装状态后重试。");}
                finally{try{await readyFilePoll.ConfigureAwait(false);}catch(OperationCanceledException){}TryDeleteReadyFile(readyFile);}
                if(port is <1 or >65535)throw new InvalidOperationException("Hermes 本地后端返回了无效端口。");
                _process=process;
                _sessionToken=token;
                _connection=new HermesConnectionInfo(installation,port,new Uri($"http://127.0.0.1:{port}/",UriKind.Absolute));
                return _connection;
            }
            catch
            {
                TryDeleteReadyFile(readyFile);
                KillOwnedProcess(process);
                process.Dispose();
                throw;
            }
        }
        finally{_gate.Release();}
    }

    internal string GetSessionToken()
    {
        if(_connection is null||string.IsNullOrEmpty(_sessionToken))throw new InvalidOperationException("Hermes 本地后端尚未连接。");
        return _sessionToken;
    }

    internal static ProcessStartInfo CreateStartInfo(HermesInstallation installation,string sessionToken,string? readyFile=null)
    {
        ArgumentNullException.ThrowIfNull(installation);
        ArgumentException.ThrowIfNullOrWhiteSpace(sessionToken);
        // If Hermes has no configured terminal.cwd, its agent falls back to
        // the serve process working directory. Keep that fallback in MewuAI's
        // own data tree so tools can never default to the Hermes installation
        // or this source checkout.
        var workspace=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","HermesWorkspace");
        Directory.CreateDirectory(workspace);
        var start=new ProcessStartInfo
        {
            FileName=installation.ExecutablePath,
            WorkingDirectory=workspace,
            UseShellExecute=false,
            CreateNoWindow=true,
            WindowStyle=ProcessWindowStyle.Hidden,
            RedirectStandardOutput=true,
            RedirectStandardError=true,
            StandardOutputEncoding=System.Text.Encoding.UTF8,
            StandardErrorEncoding=System.Text.Encoding.UTF8
        };
        start.ArgumentList.Add("serve");
        start.ArgumentList.Add("--host");
        start.ArgumentList.Add("127.0.0.1");
        start.ArgumentList.Add("--port");
        start.ArgumentList.Add("0");
        start.Environment.Remove("HERMES_DESKTOP");
        start.Environment["HERMES_HOME"]=installation.HomePath;
        start.Environment["HERMES_DASHBOARD_SESSION_TOKEN"]=sessionToken;
        start.Environment["HERMES_PARENT_PID"]=Environment.ProcessId.ToString(System.Globalization.CultureInfo.InvariantCulture);
        start.Environment["PYTHONUTF8"]="1";
        if(!string.IsNullOrWhiteSpace(readyFile))start.Environment["HERMES_DESKTOP_READY_FILE"]=Path.GetFullPath(readyFile);
        else start.Environment.Remove("HERMES_DESKTOP_READY_FILE");
        return start;
    }

    internal static bool TryParseReadyLine(string? line,out int port)
    {
        port=0;
        if(string.IsNullOrWhiteSpace(line))return false;
        var match=ReadyLineRegex().Match(line.Trim());
        return match.Success&&int.TryParse(match.Groups[1].Value,System.Globalization.NumberStyles.None,System.Globalization.CultureInfo.InvariantCulture,out port)&&port is >=1 and <=65535;
    }

    private static string CreateSessionToken()
    {
        var bytes=RandomNumberGenerator.GetBytes(32);
        try{return Convert.ToBase64String(bytes).TrimEnd('=').Replace('+','-').Replace('/','_');}
        finally{CryptographicOperations.ZeroMemory(bytes);}
    }

    private static async Task PollReadyFileAsync(string path,TaskCompletionSource<int> ready,CancellationToken cancellationToken)
    {
        while(!ready.Task.IsCompleted)
        {
            cancellationToken.ThrowIfCancellationRequested();
            try
            {
                if(File.Exists(path))
                {
                    await using var stream=new FileStream(path,FileMode.Open,FileAccess.Read,FileShare.ReadWrite|FileShare.Delete);
                    if(stream.Length>4096)throw new InvalidDataException("Hermes ready 文件超过安全上限。");
                    var buffer=new byte[checked((int)Math.Min(stream.Length+1,4097))];
                    var count=0;
                    while(count<buffer.Length)
                    {
                        var read=await stream.ReadAsync(buffer.AsMemory(count),cancellationToken).ConfigureAwait(false);
                        if(read==0)break;
                        count+=read;
                    }
                    if(count>4096)throw new InvalidDataException("Hermes ready 文件超过安全上限。");
                    using var document=JsonDocument.Parse(buffer.AsMemory(0,count),new JsonDocumentOptions{MaxDepth=8});
                    if(document.RootElement.TryGetProperty("port",out var portElement)&&portElement.TryGetInt32(out var port)&&port is >=1 and <=65535)
                    {ready.TrySetResult(port);return;}
                }
            }
            catch(Exception ex)when(ex is IOException or UnauthorizedAccessException or JsonException){}
            await Task.Delay(100,cancellationToken).ConfigureAwait(false);
        }
    }

    private static void TryDeleteReadyFile(string path){try{File.Delete(path);}catch(Exception ex)when(ex is IOException or UnauthorizedAccessException){}}

    private async Task StopOwnedProcessAsync()
    {
        var process=Interlocked.Exchange(ref _process,null);
        _connection=null;
        _sessionToken=null;
        if(process is null)return;
        try
        {
            if(!process.HasExited)
            {
                KillOwnedProcess(process);
                using var timeout=new CancellationTokenSource(ShutdownTimeout);
                try{await process.WaitForExitAsync(timeout.Token).ConfigureAwait(false);}catch(OperationCanceledException){}
            }
        }
        finally{process.Dispose();}
    }

    private static void KillOwnedProcess(Process process)
    {
        try{if(!process.HasExited)process.Kill(entireProcessTree:true);}catch(Exception ex)when(ex is InvalidOperationException or System.ComponentModel.Win32Exception or NotSupportedException){}
    }

    private static int SafeExitCode(Process process){try{return process.ExitCode;}catch{return -1;}}

    public void Dispose()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        try{StopOwnedProcessAsync().GetAwaiter().GetResult();}catch{}
        _gate.Dispose();
    }

    public async ValueTask DisposeAsync()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        try{await StopOwnedProcessAsync().ConfigureAwait(false);}catch{}
        _gate.Dispose();
    }

    [GeneratedRegex("^HERMES_BACKEND_READY\\s+port=(\\d+)$",RegexOptions.CultureInvariant)]
    private static partial Regex ReadyLineRegex();
}
