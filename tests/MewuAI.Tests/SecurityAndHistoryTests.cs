using System.Text.Json;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class SecurityAndHistoryTests
{
    [Fact]
    public void PrivacyLoggerRedactsTokenSecretPasswordAndAuthorizationValues()
    {
        var root=TestDirectory();
        try
        {
            new PrivacyLogger(root).Error("Provider",new InvalidOperationException("Authorization: Bearer auth-value access_token=token-value secret=secret-value password=password-value"));
            var text=File.ReadAllText(Directory.GetFiles(root,"*.log").Single());
            Assert.DoesNotContain("auth-value",text);Assert.DoesNotContain("token-value",text);Assert.DoesNotContain("secret-value",text);Assert.DoesNotContain("password-value",text);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void PrivacyLoggerRedactsSensitiveValuesEmbeddedInStackTrace()
    {
        var root=TestDirectory();
        try
        {
            const string stack="at Provider.Send(String url = \"https://example.invalid/chat?api_key=stack-api-key&tenant=demo\", Authorization = \"Bearer stack-bearer-token\", password = \"stack password value\", token = \"escaped \\\"secret\\\" value\") in C:\\Users\\Test\\provider.cs:line 42";
            new PrivacyLogger(root).Error("Provider",new SyntheticStackTraceException(stack));
            var text=File.ReadAllText(Directory.GetFiles(root,"*.log").Single());

            Assert.DoesNotContain("stack-api-key",text);
            Assert.DoesNotContain("stack-bearer-token",text);
            Assert.DoesNotContain("stack password value",text);
            Assert.DoesNotContain("escaped \\\"secret\\\" value",text);
            Assert.Contains("tenant=demo",text);
            Assert.Contains("provider.cs:line 42",text);
            Assert.Contains("[REDACTED]",text);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task ConcurrentHistoryWritesRemainOneValidJsonObjectPerLine()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"history.jsonl");var service=new ConversationHistoryService(path);
            await Task.WhenAll(Enumerable.Range(0,40).Select(index=>service.AppendAsync("provider","model",$"prompt-{index}",$"answer-{index}",TestContext.Current.CancellationToken)));
            var lines=await File.ReadAllLinesAsync(path,TestContext.Current.CancellationToken);
            Assert.Equal(40,lines.Length);Assert.All(lines,line=>{using var document=JsonDocument.Parse(line);Assert.Equal(JsonValueKind.Object,document.RootElement.ValueKind);});
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task PreCanceledHistoryWriteDoesNotCreatePartialRecord()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"history.jsonl");var service=new ConversationHistoryService(path);
            using var cancellation=new CancellationTokenSource();cancellation.Cancel();
            await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>service.AppendAsync("provider","model","prompt","answer",cancellation.Token));
            Assert.False(File.Exists(path));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task HistoryWriteCanceledAfterGateAcquisitionDoesNotCommitRecord()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"history.jsonl");
            var gateAcquired=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var allowCommitCheck=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var service=new ConversationHistoryService(path,null,async _=>
            {
                gateAcquired.TrySetResult();
                await allowCommitCheck.Task;
            });
            using var cancellation=new CancellationTokenSource();
            var append=service.AppendAsync("provider","model","prompt","answer",cancellation.Token);
            await gateAcquired.Task.WaitAsync(TestContext.Current.CancellationToken);

            cancellation.Cancel();
            allowCommitCheck.TrySetResult();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>append);
            Assert.False(File.Exists(path));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task ConcurrentCrashLogWritesDoNotLoseEntries()
    {
        var root=TestDirectory();
        try
        {
            var logger=new PrivacyLogger(root);
            await Task.WhenAll(Enumerable.Range(0,30).Select(index=>Task.Run(()=>logger.Error("Concurrent",new InvalidOperationException($"failure-{index}")),TestContext.Current.CancellationToken)));
            var text=string.Join(Environment.NewLine,Directory.GetFiles(root,"*.log").Select(File.ReadAllText));
            foreach(var index in Enumerable.Range(0,30))Assert.Contains($"failure-{index}",text,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    private static string TestDirectory(){var path=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(path);return path;}

    private sealed class SyntheticStackTraceException(string stackTrace):Exception("synthetic failure")
    {
        public override string? StackTrace { get; }=stackTrace;
        }

    [Fact]
    public async Task HistoryReadReturnsRecentValidRecordsAndSkipsCorruptLines()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"history.jsonl");
            await File.WriteAllLinesAsync(path,[
                "{not-json}",
                JsonSerializer.Serialize(new {timestamp=DateTimeOffset.UtcNow.AddMinutes(-2),provider="Hermes · default",model="MiniMax-M3",prompt="older",answer="older answer"}),
                JsonSerializer.Serialize(new {timestamp=DateTimeOffset.UtcNow.AddMinutes(-1),provider="Hermes · default",model="MiniMax-M3",prompt="newer",answer="newer answer"}),
                JsonSerializer.Serialize(new {timestamp=DateTimeOffset.UtcNow,provider="",model="MiniMax-M3",prompt="invalid",answer="invalid"})
            ],TestContext.Current.CancellationToken);

            var records=await new ConversationHistoryService(path,null).ReadRecentAsync(10,TestContext.Current.CancellationToken);
            Assert.Equal(2,records.Count);
            Assert.Equal("older",records[0].Prompt);
            Assert.Equal("newer",records[1].Prompt);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task HistoryReadIsBoundedToRequestedMostRecentRecords()
    {
        var root=TestDirectory();
        try
        {
            var path=Path.Combine(root,"history.jsonl");var service=new ConversationHistoryService(path);
            foreach(var index in Enumerable.Range(0,8))
                await service.AppendAsync("provider","model",$"prompt-{index}",$"answer-{index}",TestContext.Current.CancellationToken);

            var records=await service.ReadRecentAsync(3,TestContext.Current.CancellationToken);
            Assert.Equal(3,records.Count);
            Assert.Equal(["prompt-5","prompt-6","prompt-7"],records.Select(record=>record.Prompt).ToArray());
        }
        finally{Directory.Delete(root,true);}
    }

}
