// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections.Concurrent;
using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace mewu_ai_Assistant.Services;

internal sealed record CodexModelOption(string Model,string DisplayName,bool IsDefault,bool SupportsImage,string DefaultEffort,IReadOnlyList<string> Efforts)
{
    public override string ToString()=>DisplayName;
}

/// <summary>Official app-server stdio transport. Auth stays in Codex; no token files are read.</summary>
internal sealed class CodexAppServer : IAsyncDisposable
{
    private readonly Process _process;
    private readonly CancellationTokenSource _lifetime=new();
    private readonly SemaphoreSlim _writeGate=new(1,1);
    private readonly ConcurrentDictionary<int,TaskCompletionSource<JsonElement>> _pending=new();
    private readonly Task _reader,_stderr;
    private int _nextId;
    internal event Action<string,JsonElement>? Notification;
    internal Task Completion=>_reader;
    internal string WorkingDirectory {get;}
    private readonly bool _videoTools;

    private CodexAppServer(Process process,string directory,bool videoTools)
    {
        _process=process;WorkingDirectory=directory;_videoTools=videoTools;
        _reader=ReadAsync();_stderr=DrainErrorsAsync();
    }

    internal static string? Discover()
    {
        var local=Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        var candidates=new List<string>();
        foreach(var entry in (Environment.GetEnvironmentVariable("PATH")??"").Split(Path.PathSeparator))
        {
            var directory=entry.Trim().Trim('"');
            if(Path.IsPathFullyQualified(directory))candidates.Add(Path.Combine(directory,"codex.exe"));
        }
        foreach(var product in new[]{"Codex","ChatGPT"})
        {
            var root=Path.Combine(local,"OpenAI",product,"bin");
            candidates.Add(Path.Combine(root,"codex.exe"));
            try
            {
                if(Directory.Exists(root))candidates.AddRange(new DirectoryInfo(root).EnumerateDirectories().Take(64)
                    .OrderByDescending(item=>item.LastWriteTimeUtc).Select(item=>Path.Combine(item.FullName,"codex.exe")));
            }
            catch(Exception ex)when(ex is IOException or UnauthorizedAccessException){}
        }
        // Official npm Windows package layout; do not execute .cmd shims or a shell.
        candidates.Add(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),"npm","node_modules","@openai","codex","node_modules","@openai","codex-win32-x64","vendor","x86_64-pc-windows-msvc","codex","codex.exe"));
        return candidates.FirstOrDefault(File.Exists);
    }

    internal static async Task<CodexAppServer> StartAsync(CancellationToken token,bool videoTools=false)
    {
        token.ThrowIfCancellationRequested();
        var executable=Discover()??throw new InvalidOperationException("未找到本机 Codex，请先安装并登录官方 ChatGPT 桌面应用或 Codex CLI。");
        var directory=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","CodexWorkspace",Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var info=new ProcessStartInfo(executable){UseShellExecute=false,CreateNoWindow=true,RedirectStandardInput=true,RedirectStandardOutput=true,RedirectStandardError=true,WorkingDirectory=directory};
        info.ArgumentList.Add("app-server");info.ArgumentList.Add("--listen");info.ArgumentList.Add("stdio://");
        foreach(var pair in SafeConfig(videoTools))
        {
            info.ArgumentList.Add("-c");info.ArgumentList.Add(pair.Key+"="+JsonSerializer.Serialize(pair.Value));
        }
        var process=Process.Start(info)??throw new InvalidOperationException("无法启动本机 Codex。");
        var server=new CodexAppServer(process,directory,videoTools);
        try
        {
            await server.InvokeAsync("initialize",new{clientInfo=new{name="mewu_ai",title="MewuAI",version=typeof(CodexAppServer).Assembly.GetName().Version?.ToString(3)??"0.0.0"}},token).ConfigureAwait(false);
            await server.WriteAsync(new{method="initialized",@params=new{}},token).ConfigureAwait(false);
            return server;
        }
        catch{await server.DisposeAsync().ConfigureAwait(false);throw;}
    }

    internal static Dictionary<string,object> SafeConfig(bool videoTools=false)=>new()
    {
        ["model_provider"]="openai",["approval_policy"]="on-request",["approvals_reviewer"]="user",["sandbox_mode"]=videoTools?"workspace-write":"read-only",
        ["history.persistence"]="none",["project_doc_max_bytes"]=0,["web_search"]="disabled",
        ["features.shell_tool"]=videoTools,["features.unified_exec"]=videoTools,["features.apps"]=false,
        ["features.plugins"]=false,["features.memories"]=false,["features.js_repl"]=false,
        ["features.multi_agent"]=false,["features.remote_control"]=false,["features.tool_suggest"]=false,
        ["features.hooks"]=false,["features.image_generation"]=false,["features.remote_plugin"]=false,
        ["sandbox_workspace_write.network_access"]=false,
        ["tools.view_image"]=videoTools,["features.view_image"]=videoTools,["memories.generate_memories"]=false,["memories.use_memories"]=false
    };

    internal async Task<IReadOnlyList<CodexModelOption>> ReadModelsAsync(CancellationToken token)
    {
        var account=await InvokeAsync("account/read",new{refreshToken=false},token).ConfigureAwait(false);
        EnsureChatGptAccount(account);
        var models=new List<CodexModelOption>();string? cursor=null;
        for(var page=0;page<8;page++)
        {
            var result=await InvokeAsync("model/list",new{limit=100,includeHidden=false,cursor},token).ConfigureAwait(false);
            foreach(var item in result.GetProperty("data").EnumerateArray())
            {
                if(models.Count>=512)throw new InvalidDataException("Codex 模型目录过大。");
                var id=Text(item,"model");
                if(string.IsNullOrWhiteSpace(id)||models.Any(model=>model.Model==id))continue;
                var efforts=item.GetProperty("supportedReasoningEfforts").EnumerateArray().Select(value=>Text(value,"reasoningEffort")).Where(value=>value.Length>0).ToArray();
                var image=item.TryGetProperty("inputModalities",out var modalities)&&modalities.EnumerateArray().Any(value=>value.GetString()=="image");
                models.Add(new(id,Text(item,"displayName"),item.TryGetProperty("isDefault",out var defaultValue)&&defaultValue.ValueKind==JsonValueKind.True,image,Text(item,"defaultReasoningEffort"),efforts));
            }
            cursor=Text(result,"nextCursor");if(cursor.Length==0)return models;
        }
        throw new InvalidDataException("Codex 模型目录分页未结束，请升级官方客户端后重试。");
    }

    internal static void EnsureChatGptAccount(JsonElement result)
    {
        if(!result.TryGetProperty("account",out var account)||account.ValueKind!=JsonValueKind.Object)
            throw new InvalidOperationException("Codex 尚未登录，请在官方 ChatGPT 桌面应用或 Codex CLI 中完成登录，再重新检测。");
        if(Text(account,"type")!="chatgpt")throw new InvalidOperationException("此入口需要 ChatGPT 账号登录；当前 Codex 使用其他认证方式。请在官方客户端切换为 ChatGPT 登录。");
    }

    internal async Task<Dictionary<string,object>> ReadIsolatedThreadConfigAsync(CancellationToken token)
    {
        var config=SafeConfig(_videoTools);
        // Disable every configured MCP server structurally. Never return/log its credentials.
        var result=await InvokeAsync("config/read",new{includeLayers=false},token).ConfigureAwait(false);
        if(result.GetProperty("config").TryGetProperty("mcp_servers",out var servers)&&servers.ValueKind==JsonValueKind.Object)
            foreach(var server in servers.EnumerateObject())
            {
                if(server.Name.Length==0||server.Name.Any(character=>!char.IsAsciiLetterOrDigit(character)&&character is not '_' and not '-'))
                    throw new InvalidOperationException("本机 Codex 的 MCP 名称无法安全隔离，请使用仅含字母、数字、下划线或短横线的名称。");
                config[$"mcp_servers.{server.Name}.enabled"]=false;
            }
        return config;
    }

    internal async Task<JsonElement> InvokeAsync(string method,object parameters,CancellationToken token)
    {
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(token,_lifetime.Token);
        timeout.CancelAfter(TimeSpan.FromSeconds(35));
        var id=Interlocked.Increment(ref _nextId);
        var completion=new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        _pending[id]=completion;
        try
        {
            if(_reader.IsCompleted)throw new IOException("Codex 后台已退出，请重新检测连接。");
            await WriteAsync(new{id,method,@params=parameters},timeout.Token).ConfigureAwait(false);
            return await completion.Task.WaitAsync(timeout.Token).ConfigureAwait(false);
        }
        catch(OperationCanceledException)when(!token.IsCancellationRequested){throw new IOException("Codex 本机接口超时或已断开，请重新检测连接。");}
        finally{_pending.TryRemove(id,out _);}
    }

    private async Task WriteAsync(object message,CancellationToken token)
    {
        var bytes=JsonSerializer.SerializeToUtf8Bytes(message);
        try
        {
            if(bytes.Length>64*1024*1024)throw new InvalidOperationException("Codex 请求超过 64 MiB 限制。");
            await _writeGate.WaitAsync(token).ConfigureAwait(false);
            try
            {
                await _process.StandardInput.BaseStream.WriteAsync(bytes,token).ConfigureAwait(false);
                await _process.StandardInput.BaseStream.WriteAsync(new byte[]{10},token).ConfigureAwait(false);
                await _process.StandardInput.BaseStream.FlushAsync(token).ConfigureAwait(false);
            }
            finally{_writeGate.Release();}
        }
        finally{CryptographicOperations.ZeroMemory(bytes);}
    }

    private async Task ReadAsync()
    {
        try
        {
            await ReadMessagesAsync(_process.StandardOutput.BaseStream,async message=>
            {
                if(message.TryGetProperty("method",out var methodValue))
                {
                    if(message.TryGetProperty("id",out var serverId))
                    {
                        // This adapter only answers supplied text/images. Never approve tools,
                        // credentials or computer access on behalf of the screenshot user.
                        await WriteAsync(new{id=serverId.Clone(),error=new{code=-32601,message="This client does not support tool execution or interactive approvals."}},_lifetime.Token).ConfigureAwait(false);
                    }
                    else if(message.TryGetProperty("params",out var parameters))Notification?.Invoke(methodValue.GetString()??"",parameters);
                }
                else if(message.TryGetProperty("id",out var id)&&id.TryGetInt32(out var number)&&_pending.TryRemove(number,out var completion))
                {
                    if(message.TryGetProperty("error",out var error))
                    {
                        var code=error.TryGetProperty("code",out var errorCode)&&errorCode.TryGetInt32(out var numberCode)?numberCode:0;
                        completion.TrySetException(new InvalidOperationException($"Codex 拒绝了接口请求（代码 {code}），请检查登录、模型与官方客户端版本。"));
                    }
                    else if(message.TryGetProperty("result",out var result))completion.TrySetResult(result.Clone());
                    else completion.TrySetException(new InvalidDataException("Codex 返回了不完整的接口响应。"));
                }
            },_lifetime.Token).ConfigureAwait(false);
        }
        catch(Exception ex)when(ex is IOException or OperationCanceledException or JsonException or InvalidOperationException){}
        finally{foreach(var pending in _pending.Values)pending.TrySetException(new IOException("Codex 后台连接中断，未完成的回答已取消。"));}
    }

    internal static async Task ReadMessagesAsync(Stream stream,Func<JsonElement,Task> receive,CancellationToken token)
    {
        var buffer=new byte[8192];using var line=new MemoryStream();
        try
        {
            int read;
            while((read=await stream.ReadAsync(buffer,token).ConfigureAwait(false))>0)
            {
                var start=0;
                for(var index=0;index<read;index++)
                {
                    if(buffer[index]!=10)continue;
                    Append(index-start);
                    if(line.Length>0){using var json=JsonDocument.Parse(line.GetBuffer().AsMemory(0,(int)line.Length));await receive(json.RootElement).ConfigureAwait(false);}
                    CryptographicOperations.ZeroMemory(line.GetBuffer().AsSpan(0,(int)line.Length));line.SetLength(0);start=index+1;
                }
                Append(read-start);
                void Append(int count)
                {
                    if(line.Length+count>4*1024*1024)throw new InvalidDataException("Codex 单条响应超过安全限制。");
                    line.Write(buffer,start,count);
                }
            }
            if(line.Length>0)throw new InvalidDataException("Codex 响应被截断。");
        }
        finally{CryptographicOperations.ZeroMemory(buffer);CryptographicOperations.ZeroMemory(line.GetBuffer());}
    }

    private async Task DrainErrorsAsync()
    {
        var buffer=new char[2048];
        try{while(await _process.StandardError.ReadAsync(buffer.AsMemory(),_lifetime.Token).ConfigureAwait(false)>0)Array.Clear(buffer);}
        catch(Exception ex)when(ex is IOException or OperationCanceledException){}
        finally{Array.Clear(buffer);}
    }

    internal static string Text(JsonElement value,string property)=>value.ValueKind==JsonValueKind.Object&&value.TryGetProperty(property,out var item)&&item.ValueKind==JsonValueKind.String?item.GetString()??"":"";

    internal void CleanWorkspace()
    {
        var root=Path.GetFullPath(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","CodexWorkspace"))+Path.DirectorySeparatorChar;
        var path=Path.GetFullPath(WorkingDirectory);
        if(!path.StartsWith(root,StringComparison.OrdinalIgnoreCase)||Path.GetDirectoryName(path)+Path.DirectorySeparatorChar!=root)return;
        try{TempMediaRegistry.Shared.TryExecuteIfUnleased(path,true,()=>{if(Directory.Exists(path))Directory.Delete(path,true);});}
        catch(Exception ex)when(ex is IOException or UnauthorizedAccessException){}
    }

    public async ValueTask DisposeAsync()
    {
        await _lifetime.CancelAsync().ConfigureAwait(false);
        try{if(!_process.HasExited)_process.Kill(entireProcessTree:true);}catch(Exception ex)when(ex is InvalidOperationException or System.ComponentModel.Win32Exception){}
        try{await Task.WhenAll(_reader,_stderr,_process.WaitForExitAsync()).WaitAsync(TimeSpan.FromSeconds(5)).ConfigureAwait(false);}catch(Exception ex)when(ex is TimeoutException or IOException or InvalidOperationException){}
        _process.Dispose();_lifetime.Dispose();CleanWorkspace();
    }
}
