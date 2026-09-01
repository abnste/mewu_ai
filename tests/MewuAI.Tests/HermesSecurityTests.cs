using System.Diagnostics;
using System.Reflection;
using System.Runtime.CompilerServices;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

/// <summary>
/// Security and lifecycle contracts for the local Hermes bridge.  None of
/// these tests starts or connects to a real Hermes installation.
/// </summary>
public sealed class HermesSecurityTests
{
    [Fact]
    public void BackendLaunchUsesAnExactExecutableAndLoopbackEphemeralPort()
    {
        var root=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));
        var installation=new HermesInstallation(
            root,
            Path.Combine(root,"hermes-agent"),
            Path.Combine(root,"bin","hermes.exe"),
            Path.Combine(root,"config.yaml"));
        var readyFile=Path.Combine(Path.GetTempPath(),$"mewu-hermes-ready-{Guid.NewGuid():N}.json");

        var start=HermesBackendService.CreateStartInfo(installation,"unit-test-token",readyFile);

        Assert.Equal(installation.ExecutablePath,start.FileName);
        Assert.False(start.UseShellExecute);
        Assert.True(start.CreateNoWindow);
        Assert.True(start.RedirectStandardOutput);
        Assert.True(start.RedirectStandardError);
        Assert.Equal(["serve","--host","127.0.0.1","--port","0"],start.ArgumentList);
        Assert.DoesNotContain("--insecure",start.ArgumentList);
        Assert.Equal(installation.HomePath,start.Environment["HERMES_HOME"]);
        Assert.Equal("unit-test-token",start.Environment["HERMES_DASHBOARD_SESSION_TOKEN"]);
        Assert.Equal(Environment.ProcessId.ToString(System.Globalization.CultureInfo.InvariantCulture),start.Environment["HERMES_PARENT_PID"]);
        Assert.Equal(Path.GetFullPath(readyFile),start.Environment["HERMES_DESKTOP_READY_FILE"]);
        Assert.False(start.Environment.ContainsKey("HERMES_DESKTOP"));
        Assert.False(Path.GetFullPath(start.WorkingDirectory).StartsWith(Path.GetFullPath(root),StringComparison.OrdinalIgnoreCase));
    }

    [Theory]
    [InlineData("HERMES_BACKEND_READY port=1",1)]
    [InlineData("HERMES_BACKEND_READY port=65535",65535)]
    public void ReadyParserAcceptsOnlyAnExactBoundedSentinel(string line,int expected)
    {
        Assert.True(HermesBackendService.TryParseReadyLine(line,out var port));
        Assert.Equal(expected,port);
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData(" HERMES_BACKEND_READY port=123 extra")]
    [InlineData("prefix HERMES_BACKEND_READY port=123")]
    [InlineData("HERMES_BACKEND_READY port=0")]
    [InlineData("HERMES_BACKEND_READY port=65536")]
    [InlineData("HERMES_DASHBOARD_READY port=123")]
    public void ReadyParserRejectsAmbiguousOrOutOfRangeInput(string? line)
    {
        Assert.False(HermesBackendService.TryParseReadyLine(line,out _));
    }

    [Fact]
    public void BackendColdStartBudgetMatchesTheOfficialWindowsContract()
    {
        var field=typeof(HermesBackendService).GetField("StartupTimeout",BindingFlags.Static|BindingFlags.NonPublic);
        var timeout=Assert.IsType<TimeSpan>(field?.GetValue(null));

        Assert.True(timeout>=TimeSpan.FromSeconds(90),$"Hermes cold-start budget was only {timeout}.");
    }

    [Fact]
    public void BackendSessionTokensAreRandomAndUrlSafe()
    {
        var method=PrivateStaticMethod(typeof(HermesBackendService),"CreateSessionToken");
        var tokens=Enumerable.Range(0,64)
            .Select(_=>Assert.IsType<string>(method.Invoke(null,null)))
            .ToArray();

        Assert.Equal(tokens.Length,tokens.Distinct(StringComparer.Ordinal).Count());
        Assert.All(tokens,token=>
        {
            Assert.True(token.Length>=43);
            Assert.DoesNotContain("+",token,StringComparison.Ordinal);
            Assert.DoesNotContain("/",token,StringComparison.Ordinal);
            Assert.DoesNotContain("=",token,StringComparison.Ordinal);
            Assert.All(token,character=>Assert.True(char.IsAsciiLetterOrDigit(character)||character is '-' or '_'));
        });
    }

    [Theory]
    [InlineData("model --provider attacker")]
    [InlineData("model\r\n--global")]
    [InlineData("provider\t--global")]
    [InlineData("model\"quote")]
    [InlineData("model'quote")]
    [InlineData("model\\")]
    public void ModelAndProviderValuesRejectArgumentInjection(string value)
    {
        var method=PrivateStaticMethod(typeof(HermesAiProvider),"ValidateToken");
        var invocation=Assert.Throws<TargetInvocationException>(()=>method.Invoke(null,[value,"Hermes 模型"]));

        Assert.IsType<InvalidOperationException>(invocation.InnerException);
    }

    [Theory]
    [InlineData("openrouter")]
    [InlineData("anthropic/claude-sonnet-4.6")]
    [InlineData("provider:model.v1")]
    public void ModelAndProviderValuesAllowCatalogIdentifiers(string value)
    {
        PrivateStaticMethod(typeof(HermesAiProvider),"ValidateToken").Invoke(null,[value,"Hermes 模型"]);
    }

    [Fact]
    public void ReasoningEffortAllowlistIsClosed()
    {
        Assert.Equal(
            ["none","minimal","low","medium","high","xhigh","max","ultra"],
            HermesRuntimeService.ReasoningEfforts);
        var method=PrivateStaticMethod(typeof(HermesAiProvider),"NormalizeReasoning");
        var invocation=Assert.Throws<TargetInvocationException>(()=>method.Invoke(null,["unlimited"]));
        Assert.IsType<InvalidOperationException>(invocation.InnerException);
    }

    [Fact]
    public void TextAndScreenEntrypointsShareOneHermesConversation()
    {
        using var runtime=new HermesRuntimeService();
        var settings=new AppSettings{HermesEnabled=true,HermesReasoningEffort="medium"};

        var text=runtime.GetConversationProvider(HermesConversationKind.Text,()=>settings);
        var screen=runtime.GetConversationProvider(HermesConversationKind.Screen,()=>settings);

        Assert.Same(text,screen);
    }

    [Fact]
    public async Task HermesProviderClearsOwnedAttachmentEvenWhenAlreadyCancelled()
    {
        using var runtime=new HermesRuntimeService();
        using var provider=new HermesAiProvider(runtime,HermesConversationKind.Text,()=>new AppSettings());
        var data=new byte[]{11,22,33,44};
        using var cancellation=new CancellationTokenSource();
        cancellation.Cancel();

        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>provider.SendAsync(new AiRequest
        {
            Prompt="不会发送",
            Attachments=[new AiAttachment(AiAttachmentType.Image,"image/png",data,ProviderOwnsData:true)]
        },cancellation.Token));

        Assert.All(data,value=>Assert.Equal(0,value));
    }

    [Fact]
    public async Task HermesProviderPreservesBorrowedAttachmentWhenAlreadyCancelled()
    {
        using var runtime=new HermesRuntimeService();
        using var provider=new HermesAiProvider(runtime,HermesConversationKind.Text,()=>new AppSettings());
        var data=new byte[]{11,22,33,44};
        using var cancellation=new CancellationTokenSource();
        cancellation.Cancel();

        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>provider.SendAsync(new AiRequest
        {
            Prompt="不会发送",
            Attachments=[new AiAttachment(AiAttachmentType.Image,"image/png",data,ProviderOwnsData:false)]
        },cancellation.Token));

        Assert.Equal(new byte[]{11,22,33,44},data);
    }

    [Theory]
    [InlineData("audio/mpeg",".mp3")]
    [InlineData("audio/ogg",".ogg")]
    [InlineData("audio/wav",".wav")]
    [InlineData("audio/flac",".flac")]
    public void TtsDecoderAcceptsEveryFormatTheHermesEndpointCanReturn(string mime,string extension)
    {
        using var audio=HermesRuntimeService.DecodeSpeechDataUrl($"data:{mime};base64,AQID");
        Assert.Equal(mime,audio.MimeType);
        Assert.Equal(extension,audio.Extension);
        Assert.Equal(new byte[]{1,2,3},audio.Data);
    }

    [Theory]
    [InlineData("text/plain")]
    [InlineData("audio/svg+xml")]
    [InlineData("video/webm")]
    public void TtsDecoderRejectsUnexpectedMimeTypes(string mime)
    {
        Assert.Throws<NotSupportedException>(()=>HermesRuntimeService.DecodeSpeechDataUrl($"data:{mime};base64,AQID"));
    }

    [Fact]
    public void JsonRpcHasAFiniteInboundMessageBudget()
    {
        var field=typeof(HermesJsonRpcClient).GetField("MaxInboundFrameBytes",BindingFlags.Static|BindingFlags.NonPublic);
        var bytes=Assert.IsType<int>(field?.GetRawConstantValue());

        Assert.InRange(bytes,64*1024,16*1024*1024);
    }

    [Fact]
    public void JsonRpcClearsSerializedRequestsContainingPromptsAndSecrets()
    {
        var source=ReadRepositoryFile("Services","HermesJsonRpcClient.cs");

        Assert.Contains("CryptographicOperations.ZeroMemory(payload)",source,StringComparison.Ordinal);
    }

    [Fact]
    public void TtsHttpTransportCannotEscapeLoopbackThroughProxyCookiesOrRedirects()
    {
        var source=ReadRepositoryFile("Services","HermesRuntimeService.cs");

        Assert.Contains("UseProxy=false",source,StringComparison.Ordinal);
        Assert.Contains("UseCookies=false",source,StringComparison.Ordinal);
        Assert.Contains("AllowAutoRedirect=false",source,StringComparison.Ordinal);
    }

    [Fact]
    public void MissingTerminalStatusIsNeverPromotedToSuccess()
    {
        var source=ReadRepositoryFile("AI","HermesAiProvider.cs");

        Assert.DoesNotContain("ReadString(message.Payload,\"status\",\"complete\")",source,StringComparison.Ordinal);
    }

    [Fact]
    public void CancellationWaitsForTheInterruptedTerminalBeforeUnlockingTheNextTurn()
    {
        var source=ReadRepositoryFile("AI","HermesAiProvider.cs");
        var send=source.IndexOf("Task<AiResult> SendAsync",StringComparison.Ordinal);
        var drain=source.IndexOf("InterruptAndDrainAsync(turn)",send,StringComparison.Ordinal);
        var release=source.IndexOf("_turnGate.Release()",drain,StringComparison.Ordinal);
        var interrupt=source.IndexOf("session.interrupt",release,StringComparison.Ordinal);
        var terminalWait=source.IndexOf("turn.Completion.Task.WaitAsync",interrupt,StringComparison.Ordinal);

        Assert.True(drain>send,"Hermes cancellation must enter the interrupt-and-drain path.");
        Assert.True(release>drain,"The next turn must remain locked until the drain call returns.");
        Assert.True(interrupt>release,"The drain implementation must send session.interrupt.");
        Assert.True(terminalWait>interrupt,"The drain implementation must then wait for the terminal event.");
    }

    [Fact]
    public void HermesSessionsIdentifyAsDesktopWithoutEnablingTheDesktopCronTicker()
    {
        var providerSource=ReadRepositoryFile("AI","HermesAiProvider.cs");
        var backendSource=ReadRepositoryFile("Services","HermesBackendService.cs");

        Assert.Contains("[\"source\"]=\"desktop\"",providerSource,StringComparison.Ordinal);
        Assert.Contains("Environment.Remove(\"HERMES_DESKTOP\")",backendSource,StringComparison.Ordinal);
    }

    private static MethodInfo PrivateStaticMethod(Type owner,string name)=>
        owner.GetMethod(name,BindingFlags.Static|BindingFlags.NonPublic)
        ??throw new Xunit.Sdk.XunitException($"Missing security policy method {owner.Name}.{name}.");

    private static string ReadRepositoryFile(params string[] segments)
    {
        static DirectoryInfo? FindRoot(string start)
        {
            var candidate=new DirectoryInfo(start);
            while(candidate is not null&&!File.Exists(Path.Combine(candidate.FullName,"mewu_ai_Assistant.csproj")))candidate=candidate.Parent;
            return candidate;
        }
        var directory=FindRoot(SourceFilePath())??FindRoot(Environment.CurrentDirectory)??FindRoot(AppContext.BaseDirectory);
        Assert.NotNull(directory);
        return File.ReadAllText(Path.Combine([directory.FullName,..segments]));
    }

    private static string SourceFilePath([CallerFilePath] string path="")=>path;
}
