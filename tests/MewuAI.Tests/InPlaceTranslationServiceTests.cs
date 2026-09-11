// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using Xunit;

namespace MewuAI.Tests;

public sealed class InPlaceTranslationServiceTests
{
    [Theory]
    [InlineData("context")]
    [InlineData("length")]
    [InlineData("eof")]
    public async Task HttpAndSseFailuresRecoverThroughTheActualProvider(string failure)
    {
        var calls=0;
        var provider=new OpenAiCompatibleProvider(new AiProviderSettings(),"test",(_,_,_)=>
        {
            calls++;
            if(calls==1&&failure=="context")return Task.FromResult(new System.Net.Http.HttpResponseMessage(System.Net.HttpStatusCode.BadRequest)
                {Content=new System.Net.Http.StringContent("{\"error\":{\"code\":\"context_length_exceeded\"}}")});
            var content=calls==1?"{\"translations\":[":"{\"translations\":[\"译文\"]}";
            var chunk=System.Text.Json.JsonSerializer.Serialize(new{choices=new[]{new{delta=new{content},finish_reason=calls>1?"stop":failure=="length"?"length":(string?)null}}});
            return Task.FromResult(new System.Net.Http.HttpResponseMessage(System.Net.HttpStatusCode.OK)
                {Content=new System.Net.Http.StringContent("data: "+chunk+"\n\n")});
        },_=>TimeSpan.FromSeconds(10));
        var result=await new InPlaceTranslationService().TranslateAsync(provider,["one","two"],"English",null,TestContext.Current.CancellationToken);
        Assert.Equal(["译文","译文"],result);Assert.Equal(3,calls);
    }

    [Theory]
    [InlineData("{\"error\":{\"code\":\"context_length_exceeded\"}}")]
    [InlineData("{\"base_resp\":{\"status_code\":1039}}")]
    public async Task ContextRejectionIsRecoveredWithoutHistoryOrAttachments(string errorBody)
    {
        var provider=new StubProvider((_,call)=>call==1
            ?throw ProviderHttpError.Create(400,false,System.Text.Encoding.UTF8.GetBytes(errorBody))
            :new("{\"translations\":[\"译文\"]}",[]));
        var result=await new InPlaceTranslationService().TranslateAsync(provider,["first","second"],"English",null,TestContext.Current.CancellationToken);
        Assert.Equal(["译文","译文"],result);
        Assert.Equal(3,provider.Requests.Count);
        Assert.All(provider.Requests,request=>{Assert.Empty(request.History);Assert.Empty(request.Attachments);Assert.True(request.DisableReasoning);});
        Assert.True(provider.Requests[1].MaxOutputTokens<=provider.Requests[0].MaxOutputTokens);
    }

    [Fact] public async Task AuthenticationFailureIsNotRetried()
    {
        var provider=new StubProvider((_,_)=>throw ProviderHttpError.Create(401,false,"{}"u8.ToArray()));
        await Assert.ThrowsAsync<InvalidOperationException>(()=>new InPlaceTranslationService().TranslateAsync(provider,["a","b"],"English",null,TestContext.Current.CancellationToken));
        Assert.Single(provider.Requests);
    }

    [Fact] public async Task OversizedOcrRowIsSplitAndReassembledWithoutMovingFollowingRows()
    {
        var source=string.Concat(Enumerable.Repeat("A sentence with emoji 😀. ",200));
        var seen=new List<string>();
        var provider=new StubProvider((request,_)=>
        {
            using var document=System.Text.Json.JsonDocument.Parse(request.Prompt[(request.Prompt.IndexOf("source=",StringComparison.Ordinal)+7)..]);
            var values=document.RootElement.EnumerateObject().Select(property=>property.Value.GetString()!).ToArray();
            Assert.All(values,value=>Assert.True(value.Length<=1600));seen.AddRange(values);
            return new(System.Text.Json.JsonSerializer.Serialize(new{translations=values}),[]);
        });
        var result=await new InPlaceTranslationService().TranslateAsync(provider,[source,"last row"],"English",null,TestContext.Current.CancellationToken);
        Assert.Equal(2,result.Count);Assert.Equal("last row",result[1]);
        Assert.Equal(source,string.Concat(seen.Take(seen.Count-1)));
        Assert.DoesNotContain('\uFFFD',result[0]);
    }

    [Fact] public async Task PersistentTruncationOfOneLongRowRecoversWithSmallerSegments()
    {
        var provider=new StubProvider((_,call)=>call<=2?throw new InvalidDataException("truncated"):new("{\"translations\":[\"译文\"]}",[]));
        var result=await new InPlaceTranslationService().TranslateAsync(provider,[new string('a',1000)],"English",null,TestContext.Current.CancellationToken);
        Assert.Equal("译文 译文",Assert.Single(result));Assert.Equal(4,provider.Requests.Count);
    }

    [Fact] public async Task ProviderTimeoutUsesBoundedFallback()
    {
        var provider=new StubProvider((_,call)=>call==1?throw new TimeoutException():new("完整译文",[]));
        Assert.Equal("完整译文",Assert.Single(await new InPlaceTranslationService().TranslateAsync(provider,["hello"],"English",null,TestContext.Current.CancellationToken)));
        Assert.Equal(2,provider.Requests.Count);
    }

    [Fact] public void LongUnbrokenUnicodeTextIsPreservedExactly()
    {
        var original=new string('a',1599)+"😀"+new string('b',1800);
        var parts=InPlaceTranslationService.SplitText(original,1600);
        Assert.Equal(original,string.Concat(parts));
        Assert.All(parts,part=>{Assert.True(part.Length<=1600);Assert.False(char.IsHighSurrogate(part[^1]));Assert.False(char.IsLowSurrogate(part[0]));});
    }

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
