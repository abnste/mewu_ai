// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using Xunit;

namespace MewuAI.Tests;

public sealed class InPlaceTranslationServiceTests
{
    [Fact] public async Task MergedParagraphsAreSplitAndMappedBackToTheirOriginalLines()
    {
        var provider=new StubProvider((_,call)=>call switch
        {
            1=>new("{\"translations\":[\"模型合并后的一个段落\"]}",[]),
            2=>new("{\"translations\":{\"1\":\"第二行\",\"0\":\"第一行\"}}",[]),
            _=>new("{\"translations\":{\"0\":\"第三行\",\"1\":\"第四行\"}}",[])
        });
        var result=await new InPlaceTranslationService().TranslateAsync(provider,["first","second","third","fourth"],"Simplified Chinese",null,TestContext.Current.CancellationToken);
        Assert.Equal(["第一行","第二行","第三行","第四行"],result);
        Assert.Equal(3,provider.Requests.Count);
        Assert.All(provider.Requests,request=>{Assert.Contains("Simplified Chinese",request.Prompt);Assert.True(request.DisableReasoning);Assert.Empty(request.Attachments);Assert.NotNull(request.StreamingCompletionPredicate);});
    }

    [Fact] public async Task SingleLineUsesPlainTranslationAfterMalformedJsonWithoutRepeatingForever()
    {
        var provider=new StubProvider((_,call)=>new(call==1?"{\"translations\":[":"你好",[]));
        var result=await new InPlaceTranslationService().TranslateAsync(provider,["hello"],"Simplified Chinese",null,TestContext.Current.CancellationToken);
        Assert.Equal("你好",Assert.Single(result));Assert.Equal(2,provider.Requests.Count);
        Assert.Null(provider.Requests[1].StreamingCompletionPredicate);
        Assert.Contains("Simplified Chinese",provider.Requests[1].Prompt);
    }

    [Fact] public async Task TruncatedNetworkResponseIsRetriedAsSmallerRequests()
    {
        var provider=new StubProvider((_,call)=>call==1?throw new InvalidDataException("truncated"):new("{\"translations\":[\"译文\"]}",[]));
        Assert.Equal(2,(await new InPlaceTranslationService().TranslateAsync(provider,["a","b"],"English",null,TestContext.Current.CancellationToken)).Count);
        Assert.Equal(3,provider.Requests.Count);
    }

    [Fact] public async Task CancellationDiscardsLateProviderResultAndNeverStartsFallback()
    {
        using var cancellation=new CancellationTokenSource();
        var provider=new StubProvider((_,_)=>{cancellation.Cancel();return new("{\"translations\":[\"迟到\"]}",[]);});
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>new InPlaceTranslationService().TranslateAsync(provider,["text"],"English",null,cancellation.Token));
        Assert.Single(provider.Requests);
    }

    [Fact] public async Task BlankLinesDoNotNeedAProviderRequest()
    {
        var provider=new StubProvider((_,_)=>throw new InvalidOperationException());
        Assert.Equal(["",""],await new InPlaceTranslationService().TranslateAsync(provider,[""," "],"English",null,TestContext.Current.CancellationToken));
        Assert.Empty(provider.Requests);
    }

    [Fact] public async Task PersistentInvalidRepliesHaveABoundedRequestCount()
    {
        var provider=new StubProvider((_,_)=>new("",[]));
        await Assert.ThrowsAsync<InvalidDataException>(()=>new InPlaceTranslationService().TranslateAsync(provider,["a","b","c","d"],"English",null,TestContext.Current.CancellationToken));
        Assert.Equal(4,provider.Requests.Count); // four lines -> two -> one -> plain, then stop
    }

    private sealed class StubProvider(Func<AiRequest,int,AiResult> reply):IAiProvider
    {
        public List<AiRequest> Requests {get;}=[];
        public string Id=>"test";
        public AiProviderCapabilities Capabilities=>new(false,false,true,0,0,TimeSpan.Zero,new HashSet<string>());
        public Task<AiResult> SendAsync(AiRequest request,CancellationToken cancellationToken){Requests.Add(request);return Task.FromResult(reply(request,Requests.Count));}
        public Task<bool> TestConnectionAsync(CancellationToken cancellationToken)=>Task.FromResult(true);
    }
}
