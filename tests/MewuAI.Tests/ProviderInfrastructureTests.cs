using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ProviderInfrastructureTests
{
    [Fact]
    public void FreshProviderDefaultsToDomesticMiniMaxM3()
    {
        var settings=new AiProviderSettings();
        Assert.Equal("MiniMax M3",settings.Name);
        Assert.Equal("MiniMax",settings.Type);
        Assert.Equal("https://api.minimaxi.com/v1",settings.BaseUrl);
        Assert.Equal("MiniMax-M3",settings.Model);
    }

    [Fact]
    public void FinishReasonChunkIsAccumulatedAndReportedBeforeStopping()
    {
        const string line="data: {\"choices\":[{\"finish_reason\":\"stop\",\"delta\":{\"content\":\"最后一段\",\"reasoning_content\":\"最终思考\"}}]}";
        Assert.True(StreamingResponseParser.TryParse(line,out var delta,out var done));
        var progress=new InlineProgress();var accumulator=new StreamingResponseAccumulator();
        var predicateValue=string.Empty;
        Assert.True(accumulator.Accept(delta,done,progress,value=>{predicateValue=value;return false;}));
        var result=accumulator.BuildResult();
        Assert.Equal("最后一段",result.Answer);Assert.Equal("最终思考",result.Reasoning);
        Assert.Equal("最后一段",predicateValue);
        Assert.Collection(progress.Values,item=>{Assert.Equal("最后一段",item.Content);Assert.Equal("最终思考",item.ReasoningContent);});
    }

    [Fact]
    public void CumulativeReasoningDetailsOnlyReportNewTextAndKeepLatestWholeValue()
    {
        var progress=new InlineProgress();var accumulator=new StreamingResponseAccumulator();
        accumulator.Accept(new AiStreamDelta(string.Empty,"先看",true),false,progress,null);
        accumulator.Accept(new AiStreamDelta(string.Empty,"先看图片",true),false,progress,null);
        accumulator.Accept(new AiStreamDelta("答案","先看图片",true),true,progress,null);
        var result=accumulator.BuildResult();
        Assert.Equal("答案",result.Answer);Assert.Equal("先看图片",result.Reasoning);
        Assert.Collection(progress.Values,
            item=>Assert.Equal("先看",item.ReasoningContent),
            item=>Assert.Equal("图片",item.ReasoningContent),
            item=>{Assert.Equal("答案",item.Content);Assert.Empty(item.ReasoningContent);});
    }

    [Fact]
    public void ReasoningOnlyChunksDoNotRebuildAnswerForCompletionPredicate()
    {
        var calls=0;var accumulator=new StreamingResponseAccumulator();
        Assert.False(accumulator.Accept(new AiStreamDelta(string.Empty,"第一步"),false,null,_=>{calls++;return false;}));
        Assert.False(accumulator.Accept(new AiStreamDelta(string.Empty,"第二步"),false,null,_=>{calls++;return false;}));
        Assert.Equal(0,calls);
        Assert.True(accumulator.Accept(new AiStreamDelta("完成",string.Empty),true,null,value=>{calls++;Assert.Equal("完成",value);return false;}));
        Assert.Equal(1,calls);
    }

    [Fact]
    public async Task NonStreamingResponseCombinesReasoningDetailsText()
    {
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},
            "unused",
            (_,_,_)=>Task.FromResult(new HttpResponseMessage(System.Net.HttpStatusCode.OK)
            {
                Content=new StringContent("{\"choices\":[{\"message\":{\"content\":\"完成\",\"reasoning_details\":[{\"text\":\"先分析\"},{\"text\":\"再回答\"}]}}]}")
            }),
            _=>TimeSpan.FromMinutes(1));
        var result=await provider.SendAsync(new AiRequest{Prompt="test"},TestContext.Current.CancellationToken);
        Assert.Equal("完成",result.Answer);Assert.Equal("先分析再回答",result.Reasoning);
    }

    [Fact]
    public async Task MiniMaxM3ExplicitlyRequestsAdaptiveSplitReasoning()
    {
        string? requestBody=null;
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="MiniMax",BaseUrl="https://api.minimaxi.com/v1",Model="MiniMax-M3"},
            "unused",
            async (request,_,_)=>
            {
                requestBody=await request.Content!.ReadAsStringAsync();
                return new HttpResponseMessage(System.Net.HttpStatusCode.OK)
                {
                    Content=new StringContent("{\"choices\":[{\"message\":{\"content\":\"完成\"}}]}")
                };
            },
            _=>TimeSpan.FromMinutes(1));

        await provider.SendAsync(new AiRequest{Prompt="test"},TestContext.Current.CancellationToken);

        using var document=System.Text.Json.JsonDocument.Parse(requestBody!);
        var root=document.RootElement;
        Assert.True(root.GetProperty("reasoning_split").GetBoolean());
        Assert.Equal("adaptive",root.GetProperty("thinking").GetProperty("type").GetString());
    }

    [Fact]
    public void StructuredAccumulatorSuppressesInvalidJsonEnvelope()
    {
        var accumulator=new StreamingResponseAccumulator(contentIsCumulative:true,expectStructuredResponse:true);
        accumulator.Accept(new AiStreamDelta("{\"result\":\"协议错误\"}","已思考"),true,null,null);

        var result=accumulator.BuildResult();

        Assert.Empty(result.Answer);
        Assert.Equal("已思考",result.Reasoning);
    }

    [Fact]
    public async Task TestConnectionRequiresTheChallengeMarkerInsteadOfAnyNonEmptyText()
    {
        string? requestBody=null;
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},
            "unused",
            async (request,_,_) =>
            {
                requestBody=await request.Content!.ReadAsStringAsync();
                return new HttpResponseMessage(System.Net.HttpStatusCode.OK)
                {
                    Content=new StringContent("{\"choices\":[{\"message\":{\"content\":\"服务已连接\"}}]}")
                };
            },
            _=>TimeSpan.FromMinutes(1));

        var connected=await provider.TestConnectionAsync(TestContext.Current.CancellationToken);

        Assert.False(connected);
        Assert.Contains(OpenAiCompatibleProvider.ConnectionProbeMarker,requestBody,StringComparison.Ordinal);
        Assert.Contains(OpenAiCompatibleProvider.ConnectionProbePrompt,requestBody,StringComparison.Ordinal);
    }

    [Fact]
    public async Task TestConnectionAcceptsTheExactChallengeMarkerWithWhitespaceAndCaseDifferences()
    {
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},
            "unused",
            (_,_,_)=>Task.FromResult(new HttpResponseMessage(System.Net.HttpStatusCode.OK)
            {
                Content=new StringContent("{\"choices\":[{\"message\":{\"content\":\"  mewu_ok\\n\"}}]}")
            }),
            _=>TimeSpan.FromMinutes(1));

        Assert.True(await provider.TestConnectionAsync(TestContext.Current.CancellationToken));
    }

    [Fact]
    public void ProviderConstructorRejectsHeaderInjectionFromHandEditedSettings()
    {
        var settings=new AiProviderSettings{CustomHeaders=new(){{"X-Tenant","safe\r\nAuthorization: stolen"}}};
        Assert.Throws<InvalidOperationException>(()=>new OpenAiCompatibleProvider(settings,"unused"));
    }

    [Theory]
    [InlineData("https://example.com/v1","https://example.com/v1/")]
    [InlineData("http://localhost:11434/v1","http://localhost:11434/v1/")]
    [InlineData("http://127.0.0.1:8080/v1","http://127.0.0.1:8080/v1/")]
    [InlineData("http://[::1]:8080/v1","http://[::1]:8080/v1/")]
    public void EndpointPolicyAcceptsHttpsAndLoopbackHttp(string input,string expected)
    {
        Assert.Equal(expected,ProviderEndpointPolicy.NormalizeBaseUri(input).AbsoluteUri);
    }

    [Theory]
    [InlineData("http://example.com/v1")]
    [InlineData("https://user:password@example.com/v1")]
    [InlineData("https://example.com/v1?tenant=secret")]
    [InlineData("https://example.com/v1#fragment")]
    [InlineData("http://localhost.example.com/v1")]
    [InlineData("/v1")]
    [InlineData("not a URL")]
    [InlineData("ftp://example.com/v1")]
    public void EndpointPolicyRejectsUnsafeOrInvalidAddresses(string input)
    {
        Assert.Throws<InvalidOperationException>(()=>ProviderEndpointPolicy.NormalizeBaseUri(input));
        Assert.Throws<InvalidOperationException>(()=>new OpenAiCompatibleProvider(new AiProviderSettings{BaseUrl=input},"unused"));
    }

    [Fact]
    public void FactoryReturnsUnavailableInsteadOfThrowingForHandEditedUnsafeEndpoint()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));var headerCredentials=new ProviderHeaderCredentialService(credentials);
            var provider=new AiProviderSettings{Type="OpenAICompatible",BaseUrl="http://example.com/v1",Model="model",CustomHeaders=new(){{"Authorization","Bearer custom"}}};
            headerCredentials.ProtectEditableHeaders(provider);var settings=new AppSettings{Providers=[provider],DefaultProviderId=provider.Id};
            Assert.Null(new AiProviderFactory(credentials,null).Create(settings));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void FactoryFailsClosedForUnknownProviderType()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            var provider=new AiProviderSettings{Type="UnknownProtocol",BaseUrl="https://example.invalid/v1",Model="model",CredentialId="credential"};
            credentials.Save(provider.CredentialId,"secret");
            Assert.Null(new AiProviderFactory(credentials,null).Create(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id}));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void FactoryFailsClosedWithActionableErrorForMissingDefaultProvider()
    {
        var provider=new AiProviderSettings{Id="available",Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"};
        var logged=new List<(string Component,Exception Exception)>();
        var factory=new AiProviderFactory(null,(component,exception)=>logged.Add((component,exception)));
        Assert.Null(factory.Create(new AppSettings{Providers=[provider],DefaultProviderId="missing"},out var error));
        Assert.Contains("默认 AI Provider 已不存在",error,StringComparison.Ordinal);
        var entry=Assert.Single(logged);Assert.Equal("ProviderConfiguration",entry.Component);Assert.IsType<InvalidOperationException>(entry.Exception);
    }

    [Fact]
    public void FactoryFailsClosedWhenPrimaryAndHeaderAuthenticationCompete()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(Path.Combine(root,"Credentials"));
            var headerCredentials=new ProviderHeaderCredentialService(credentials);
            var provider=new AiProviderSettings{Id="provider",Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model",CredentialId="primary",CustomHeaders=new(){{"X-Api-Key","header-secret"}}};
            credentials.Save(provider.CredentialId,"primary-secret");headerCredentials.ProtectEditableHeaders(provider);
            var factory=new AiProviderFactory(credentials,null);
            Assert.Null(factory.Create(new AppSettings{Providers=[provider],DefaultProviderId=provider.Id},out var error));
            Assert.Contains("同时配置了 API Key 与认证 Header",error,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void HeaderOnlyAuthenticationDoesNotInjectAnEmptyDefaultBearer()
    {
        var settings=new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model",CustomHeaders=new(){{"X-Api-Key","header-secret"}}};
        using var request=new InspectableOpenAiProvider(settings,string.Empty).CreateRequest();
        Assert.Null(request.Headers.Authorization);
        Assert.Equal("header-secret",Assert.Single(request.Headers.GetValues("X-Api-Key")));
    }

    [Fact]
    public void RequestTimeoutPolicyAllowsMoreTimeForVideoUploads()
    {
        Assert.Equal(TimeSpan.FromMinutes(5),ProviderRequestTimeoutPolicy.For(new AiRequest{Prompt="text"}));
        Assert.Equal(TimeSpan.FromMinutes(5),ProviderRequestTimeoutPolicy.For(new AiRequest
        {
            Attachments=[new AiAttachment(AiAttachmentType.Image,"image/png",[1])]
        }));
        Assert.Equal(TimeSpan.FromMinutes(10),ProviderRequestTimeoutPolicy.For(new AiRequest
        {
            Attachments=[new AiAttachment(AiAttachmentType.Video,"video/mp4",[1])]
        }));
    }

    [Fact]
    public async Task ProviderTurnsItsBoundedDeadlineIntoAnActionableTimeout()
    {
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},
            "unused",
            async (_,_,requestToken)=>
            {
                await Task.Delay(Timeout.InfiniteTimeSpan,requestToken);
                return new HttpResponseMessage(System.Net.HttpStatusCode.OK);
            },
            _=>TimeSpan.FromMilliseconds(30));

        var error=await Assert.ThrowsAsync<TimeoutException>(()=>provider.SendAsync(
            new AiRequest{Prompt="wait"},
            TestContext.Current.CancellationToken));

        Assert.Contains("AI 请求超过",error.Message,StringComparison.Ordinal);
        Assert.IsAssignableFrom<OperationCanceledException>(error.InnerException);
    }

    [Fact]
    public async Task CallerCancellationWinsOverTheProviderDeadline()
    {
        var transportCalled=false;
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},
            "unused",
            (_,_,_)=>
            {
                transportCalled=true;
                return Task.FromResult(new HttpResponseMessage(System.Net.HttpStatusCode.OK));
            },
            _=>TimeSpan.FromMinutes(1));
        using var cancellation=new CancellationTokenSource();
        cancellation.Cancel();

        var error=await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>provider.SendAsync(
            new AiRequest{Prompt="cancel"},
            cancellation.Token));

        Assert.Equal(cancellation.Token,error.CancellationToken);
        Assert.False(transportCalled);
    }

    [Fact]
    public async Task StreamingResponseWithoutATerminalEventFailsInsteadOfAcceptingPartialText()
    {
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},
            "unused",
            (_,_,_)=>Task.FromResult(new HttpResponseMessage(System.Net.HttpStatusCode.OK)
            {
                Content=new StringContent("data: {\"choices\":[{\"finish_reason\":null,\"delta\":{\"content\":\"半段回答\"}}]}\n\n")
            }),
            _=>TimeSpan.FromMinutes(1));
        var progress=new InlineProgress();

        var error=await Assert.ThrowsAsync<InvalidDataException>(()=>provider.SendAsync(
            new AiRequest{Prompt="test",StreamingProgress=progress},
            TestContext.Current.CancellationToken));

        Assert.Contains("意外中断",error.Message,StringComparison.Ordinal);
        Assert.Collection(progress.Values,item=>Assert.Equal("半段回答",item.Content));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task ProviderRejectsOversizedDeclaredResponseBeforeReading(bool streaming)
    {
        var content=new StringContent("{}");
        content.Headers.ContentLength=OpenAiCompatibleProvider.ResponseBodySizeLimit+1;
        var provider=ProviderWithResponse(content);
        var request=new AiRequest
        {
            Prompt="test",
            StreamingProgress=streaming?new InlineProgress():null
        };

        var error=await Assert.ThrowsAsync<InvalidDataException>(()=>provider.SendAsync(request,TestContext.Current.CancellationToken));

        Assert.Contains("8 MB",error.Message,StringComparison.Ordinal);
        Assert.Contains("降低最大输出 Token",error.Message,StringComparison.Ordinal);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task ProviderRejectsOversizedChunkedResponseWhileReading(bool streaming)
    {
        var content=new StreamContent(new PatternReadStream(OpenAiCompatibleProvider.ResponseBodySizeLimit+1));
        Assert.Null(content.Headers.ContentLength);
        var provider=ProviderWithResponse(content);
        var request=new AiRequest
        {
            Prompt="test",
            StreamingProgress=streaming?new InlineProgress():null
        };

        var error=await Assert.ThrowsAsync<InvalidDataException>(()=>provider.SendAsync(request,TestContext.Current.CancellationToken));

        Assert.Contains("8 MB",error.Message,StringComparison.Ordinal);
        Assert.Contains("清理对话历史",error.Message,StringComparison.Ordinal);
    }

    [Fact]
    public void PrimaryApiKeyStillInjectsDefaultBearer()
    {
        var settings=new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"};
        using var request=new InspectableOpenAiProvider(settings,"primary-secret").CreateRequest();
        Assert.Equal("Bearer",request.Headers.Authorization?.Scheme);
        Assert.Equal("primary-secret",request.Headers.Authorization?.Parameter);
    }

    [Fact]
    public async Task MiniMaxRejectsFiftyMiBInlineVideoAfterBase64Expansion()
    {
        var root=TestDirectory();
        try
        {
            var media=Path.Combine(root,"media.mp4");
            using(var stream=new FileStream(media,FileMode.CreateNew,FileAccess.Write,FileShare.None))stream.SetLength(50L*1024*1024);
            var provider=MiniMax();
            var request=new AiRequest{Prompt="分析视频",Attachments=[new(AiAttachmentType.Video,"video/mp4",FilePath:media)]};
            var estimate=provider.EstimateRequestBodyBytes(request);
            Assert.True(estimate>MiniMaxProvider.MaxRequestBodyBytes);
            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(request,TestContext.Current.CancellationToken));
            Assert.Contains("64 MB",error.Message,StringComparison.Ordinal);
            Assert.Contains("Files API",error.Message,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task MiniMaxEstimateAllowsBase64VideoBelowAggregateBoundary()
    {
        var root=TestDirectory();
        try
        {
            var media=Path.Combine(root,"media.mp4");
            await using(var stream=new FileStream(media,FileMode.CreateNew,FileAccess.Write,FileShare.None))stream.SetLength(47L*1024*1024);
            var estimate=MiniMax().EstimateRequestBodyBytes(new AiRequest{Prompt="分析视频",Attachments=[new(AiAttachmentType.Video,"video/mp4",FilePath:media)]});
            Assert.True(estimate<MiniMaxProvider.MaxRequestBodyBytes);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void CredentialSaveUsesAtomicFinalBlobWithoutLeavingTemporaryFiles()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(root);credentials.Save("atomic","first");credentials.Save("atomic","second");
            Assert.Equal("second",credentials.Read("atomic"));
            Assert.Collection(Directory.GetFiles(root),path=>Assert.Equal(Path.Combine(root,"atomic.bin"),path));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void ApiKeyChangePolicy_PreservesReplacesAndStagesDeletionWithoutDeletingBeforeCommit()
    {
        var root=TestDirectory();
        try
        {
            var credentials=new CredentialService(root);credentials.Save("old-id","old-secret");
            var preserved=new AiProviderSettings{CredentialId="old-id"};ProviderApiKeyChangePolicy.Apply(preserved,null,false,credentials);Assert.Equal("old-id",preserved.CredentialId);Assert.Equal("old-secret",credentials.Read("old-id"));
            var replaced=new AiProviderSettings{CredentialId="old-id"};ProviderApiKeyChangePolicy.Apply(replaced,"new-secret",false,credentials);Assert.NotEqual("old-id",replaced.CredentialId);Assert.Equal("new-secret",credentials.Read(replaced.CredentialId));Assert.Equal("old-secret",credentials.Read("old-id"));
            var deleted=new AiProviderSettings{CredentialId="old-id"};ProviderApiKeyChangePolicy.Apply(deleted,null,true,credentials);Assert.Empty(deleted.CredentialId);Assert.Equal("old-secret",credentials.Read("old-id"));
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public void ApiKeyChangePolicy_DetectsCompetingHeaderAuthentication()
    {
        var provider=new AiProviderSettings{CredentialId="primary",SensitiveHeaderCredentialIds=new(){{"X-Api-Key","header"}}};
        Assert.True(ProviderApiKeyChangePolicy.HasCompetingAuthentication(provider));
        provider.CredentialId=string.Empty;
        Assert.False(ProviderApiKeyChangePolicy.HasCompetingAuthentication(provider));
    }

    [Fact]
    public async Task MiniMaxRejectsSeveralIndividuallyValidImagesOverAggregateBudget()
    {
        var root=TestDirectory();
        try
        {
            var image=Path.Combine(root,"image.png");
            await using(var stream=new FileStream(image,FileMode.CreateNew,FileAccess.Write,FileShare.None))stream.SetLength(8L*1024*1024);
            var request=new AiRequest{Attachments=Enumerable.Range(0,7).Select(_=>new AiAttachment(AiAttachmentType.Image,"image/png",FilePath:image)).ToList()};
            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>MiniMax().SendAsync(request,TestContext.Current.CancellationToken));
            Assert.Contains("64 MB",error.Message,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task MiniMaxKeepsOfficialPerAttachmentImageLimit()
    {
        var root=TestDirectory();
        try
        {
            var image=Path.Combine(root,"image.png");
            await using(var stream=new FileStream(image,FileMode.CreateNew,FileAccess.Write,FileShare.None))stream.SetLength(10L*1024*1024+1);
            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>MiniMax().SendAsync(new AiRequest{Attachments=[new(AiAttachmentType.Image,"image/png",FilePath:image)]},TestContext.Current.CancellationToken));
            Assert.Contains("10 MB",error.Message,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task ProviderRejectsMoreThanSixteenAttachmentsBeforeNetwork()
    {
        var transportCalled=false;
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},
            "unused",
            (_,_,_)=>{transportCalled=true;return Task.FromResult(new HttpResponseMessage(System.Net.HttpStatusCode.OK));},
            _=>TimeSpan.FromMinutes(1));
        var request=new AiRequest{Attachments=Enumerable.Range(0,OpenAiCompatibleProvider.AttachmentCountLimit+1).Select(_=>new AiAttachment(AiAttachmentType.Image,"image/png",[1])).ToList()};

        var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(request,TestContext.Current.CancellationToken));

        Assert.Contains("16",error.Message,StringComparison.Ordinal);Assert.False(transportCalled);
    }

    [Fact]
    public async Task GenericProviderRejectsAggregateBase64BodyBeforeReadingSparseFiles()
    {
        var root=TestDirectory();
        try
        {
            var image=Path.Combine(root,"image.png");await using(var stream=new FileStream(image,FileMode.CreateNew,FileAccess.Write,FileShare.None))stream.SetLength(13L*1024*1024);
            var request=new AiRequest{Attachments=Enumerable.Range(0,4).Select(_=>new AiAttachment(AiAttachmentType.Image,"image/png",FilePath:image)).ToList()};
            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>new OpenAiCompatibleProvider(new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},"unused").SendAsync(request,TestContext.Current.CancellationToken));
            Assert.Contains("64 MB",error.Message,StringComparison.Ordinal);Assert.Contains("Base64",error.Message,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task ProviderRejectsOversizedJsonEnvelopeBeforeSerializationOrNetwork()
    {
        var transportCalled=false;
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},
            "unused",
            (_,_,_)=>{transportCalled=true;return Task.FromResult(new HttpResponseMessage(System.Net.HttpStatusCode.OK));},
            _=>TimeSpan.FromMinutes(1));
        var request=new AiRequest{Prompt=new string('你',11*1024*1024)};
        Assert.True(provider.EstimateRequestBodyBytes(request)>OpenAiCompatibleProvider.RequestBodySizeLimit);

        var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(request,TestContext.Current.CancellationToken));

        Assert.Contains("64 MB",error.Message,StringComparison.Ordinal);Assert.False(transportCalled);
    }

    [Theory]
    [InlineData("")]
    [InlineData("plain ASCII 123")]
    [InlineData("<script>&'\"\\\u007f")]
    [InlineData("中文与😀")]
    public void JsonStringEstimateIsNeverBelowTheActualDefaultEncoderSize(string value)
    {
        var actual=System.Text.Json.JsonSerializer.SerializeToUtf8Bytes(value).LongLength;
        Assert.True(OpenAiCompatibleProvider.EstimateJsonStringBytesUpperBound(value)>=actual);
    }

    [Fact]
    public async Task VolcengineVideoUsesPerTypeLimitsAndFailsFastAfterBase64Expansion()
    {
        var root=TestDirectory();
        try
        {
            var video=Path.Combine(root,"video.mp4");await using(var stream=new FileStream(video,FileMode.CreateNew,FileAccess.Write,FileShare.None))stream.SetLength(48L*1024*1024);
            var provider=new OpenAiCompatibleProvider(new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://ark.cn-beijing.volces.com/api/plan/v3",Model="doubao-seed-2-0-pro-260215"},"unused");
            Assert.Equal(10L*1024*1024,provider.Capabilities.MaxImageSize);Assert.Equal(50L*1024*1024,provider.Capabilities.MaxVideoSize);
            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(new AiRequest{Attachments=[new(AiAttachmentType.Video,"video/mp4",FilePath:video)]},TestContext.Current.CancellationToken));
            Assert.Contains("64 MB",error.Message,StringComparison.Ordinal);Assert.Contains("47 MB",error.Message,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task VolcengineRejectsImageAboveItsTenMiBLimitBeforeReadingIt()
    {
        var root=TestDirectory();
        try
        {
            var image=Path.Combine(root,"image.png");await using(var stream=new FileStream(image,FileMode.CreateNew,FileAccess.Write,FileShare.None))stream.SetLength(10L*1024*1024+1);
            var provider=new OpenAiCompatibleProvider(new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://ark.cn-beijing.volces.com/api/plan/v3",Model="doubao-seed-2-0-pro-260215"},"unused");
            var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(new AiRequest{Attachments=[new(AiAttachmentType.Image,"image/png",FilePath:image)]},TestContext.Current.CancellationToken));
            Assert.Contains("10 MB",error.Message,StringComparison.Ordinal);Assert.Contains("图片",error.Message,StringComparison.Ordinal);
        }
        finally{Directory.Delete(root,true);}
    }

    [Fact]
    public async Task RequestHeaderFailureDoesNotFallThroughToNetworkSend()
    {
        var settings=new AiProviderSettings
        {
            Type="OpenAICompatible",
            BaseUrl="https://example.invalid/v1",
            Model="model",
            CustomHeaders=new(){{"Content-Type","application/custom"}}
        };
        var provider=new OpenAiCompatibleProvider(settings,"unused");
        var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(
            new AiRequest{Prompt="buffer must be cleared"},
            TestContext.Current.CancellationToken));
        Assert.Contains("Content-Type",error.Message,StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task ProviderClearsOwnedMemoryAttachmentAfterSuccessfulRequest()
    {
        var data=new byte[]{1,2,3,4,5};
        var provider=ProviderWithResponse(new StringContent("{\"choices\":[{\"message\":{\"content\":\"完成\"}}]}"));

        var result=await provider.SendAsync(new AiRequest
        {
            Prompt="识别图片",
            Attachments=[new AiAttachment(AiAttachmentType.Image,"image/png",data,ProviderOwnsData:true)]
        },TestContext.Current.CancellationToken);

        Assert.Equal("完成",result.Answer);
        Assert.All(data,value=>Assert.Equal(0,value));
    }

    [Fact]
    public async Task ProviderClearsOwnedMemoryAttachmentWhenValidationFails()
    {
        var data=new byte[]{6,7,8};
        var provider=ProviderWithResponse(new StringContent("{}"));

        await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(new AiRequest
        {
            Prompt="无效附件",
            Attachments=[new AiAttachment(AiAttachmentType.Image,"image/png",data,FilePath:"also-a-path",ProviderOwnsData:true)]
        },TestContext.Current.CancellationToken));

        Assert.All(data,value=>Assert.Equal(0,value));
    }

    [Fact]
    public async Task ProviderPreservesBorrowedMemoryAttachment()
    {
        var data=new byte[]{9,10,11};
        var provider=ProviderWithResponse(new StringContent("{\"choices\":[{\"message\":{\"content\":\"完成\"}}]}"));

        await provider.SendAsync(new AiRequest
        {
            Prompt="识别图片",
            Attachments=[new AiAttachment(AiAttachmentType.Image,"image/png",data,ProviderOwnsData:false)]
        },TestContext.Current.CancellationToken);

        Assert.Equal(new byte[]{9,10,11},data);
    }

    private static MiniMaxProvider MiniMax()=>new(new AiProviderSettings{Type="MiniMax",BaseUrl="https://api.minimaxi.com/v1",Model="MiniMax-M3"},"unused");
    private static OpenAiCompatibleProvider ProviderWithResponse(HttpContent content)=>new(
        new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="model"},
        "unused",
        (_,_,_)=>Task.FromResult(new HttpResponseMessage(System.Net.HttpStatusCode.OK){Content=content}),
        _=>TimeSpan.FromMinutes(1));
    private static string TestDirectory(){var path=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));Directory.CreateDirectory(path);return path;}

    private sealed class InspectableOpenAiProvider(AiProviderSettings settings,string key):OpenAiCompatibleProvider(settings,key)
    {
        public HttpRequestMessage CreateRequest()=>Create(HttpMethod.Post,"chat/completions");
    }

    private sealed class InlineProgress:IProgress<AiStreamDelta>
    {
        public List<AiStreamDelta> Values { get; }=[];
        public void Report(AiStreamDelta value)=>Values.Add(value);
    }

    private sealed class PatternReadStream(long length):Stream
    {
        private long _position;

        public override bool CanRead=>true;
        public override bool CanSeek=>false;
        public override bool CanWrite=>false;
        public override long Length=>throw new NotSupportedException();
        public override long Position { get=>throw new NotSupportedException();set=>throw new NotSupportedException(); }
        public override void Flush()=>throw new NotSupportedException();
        public override long Seek(long offset,SeekOrigin origin)=>throw new NotSupportedException();
        public override void SetLength(long value)=>throw new NotSupportedException();
        public override void Write(byte[] buffer,int offset,int count)=>throw new NotSupportedException();
        public override int Read(byte[] buffer,int offset,int count)=>Read(buffer.AsSpan(offset,count));

        public override int Read(Span<byte> buffer)
        {
            if(_position>=length||buffer.Length==0)return 0;
            var count=(int)Math.Min(buffer.Length,length-_position);
            for(var index=0;index<count;index++)
                buffer[index]=(_position+index+1)%1024==0?(byte)'\n':(byte)'x';
            _position+=count;
            return count;
        }

        public override Task<int> ReadAsync(byte[] buffer,int offset,int count,CancellationToken cancellationToken)=>
            ReadAsync(buffer.AsMemory(offset,count),cancellationToken).AsTask();

        public override ValueTask<int> ReadAsync(Memory<byte> buffer,CancellationToken cancellationToken=default)
        {
            cancellationToken.ThrowIfCancellationRequested();
            return ValueTask.FromResult(Read(buffer.Span));
        }
    }
}
