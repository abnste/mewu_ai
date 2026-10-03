// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Net.Http;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class McpImageServiceTests
{
    private static CancellationToken TestToken=>TestContext.Current.CancellationToken;
    private static AppSettings Feishu()=>new(){FeishuEnabled=true,FeishuAppId="fixture-app",FeishuTargetType="chat_id",FeishuTargetId="fixture-chat"};
    private static AppSettings DingTalk()=>new(){DingTalkEnabled=true,DingTalkAppKey="fixture-app",DingTalkAgentId="123",DingTalkTargetUsers="alice|bob\nalice"};
    private static AppSettings Ima()=>new(){ImaEnabled=true,ImaClientId="fixture-client",ImaKnowledgeBaseId="fixture-base",ImaKnowledgeBaseName="Fixture"};

    [Fact]
    public async Task FeishuUsesOfficialMultipartAndImageKeyAndKeepsBorrowedClientAlive()
    {
        using var handler=new FakeHandler(async(request,call,token)=>
        {
            if(call is 1 or 4)return Json("""{"code":0,"tenant_access_token":"fixture-token"}""");
            Assert.Equal("Bearer",request.Headers.Authorization?.Scheme);
            Assert.Equal("fixture-token",request.Headers.Authorization?.Parameter);
            if(call==2)
            {
                Assert.Equal("/open-apis/im/v1/images",request.RequestUri!.AbsolutePath);
                Assert.Empty(request.RequestUri.Query);
                var form=Assert.IsType<MultipartFormDataContent>(request.Content);
                var type=form.Single(x=>x.Headers.ContentDisposition!.Name!.Trim('"')=="image_type");
                Assert.Equal("message",await type.ReadAsStringAsync(token));
                var image=form.Single(x=>x.Headers.ContentDisposition!.Name!.Trim('"')=="image");
                Assert.Equal(new byte[]{1,2,3},await image.ReadAsByteArrayAsync(token));
                return Json("""{"code":0,"data":{"image_key":"fixture-image"}}""");
            }
            Assert.Equal("/open-apis/im/v1/messages",request.RequestUri!.AbsolutePath);
            Assert.Equal("?receive_id_type=chat_id",request.RequestUri.Query);
            var payload=JsonNode.Parse(await request.Content!.ReadAsStringAsync(token))!;
            Assert.Equal("fixture-chat",payload["receive_id"]!.GetValue<string>());
            var content=JsonNode.Parse(payload["content"]!.GetValue<string>())!.AsObject();
            Assert.Single(content);
            Assert.Equal("fixture-image",content["image_key"]!.GetValue<string>());
            return Json("""{"code":0,"data":{"message_id":"fixture-message"}}""");
        });
        using var client=new HttpClient(handler);
        await FeishuService.SendImageAsync(Feishu(),[1,2,3],client,"fixture-secret",TestToken);
        Assert.False(handler.Disposed);
        Assert.Equal("fixture-token",await FeishuService.GetTenantTokenAsync(Feishu(),client,"fixture-secret",TestToken));
        Assert.Equal(4,handler.Calls);
    }

    [Fact]
    public async Task FeishuListsAllPagesAndDeduplicatesChats()
    {
        using var handler=new FakeHandler((request,call,token)=>
        {
            if(call==1)return Task.FromResult(Json("""{"code":0,"tenant_access_token":"fixture-token"}"""));
            if(call==2)return Task.FromResult(Json("""{"code":0,"data":{"items":[{"chat_id":"one","name":"One"}],"has_more":true,"page_token":"a+b/="}}"""));
            Assert.Contains("page_token=a%2Bb%2F%3D",request.RequestUri!.Query);
            return Task.FromResult(Json("""{"code":0,"data":{"items":[{"chat_id":"one","name":"One"},{"chat_id":"two","name":"Two"}],"has_more":false}}"""));
        });
        using var client=new HttpClient(handler);
        var chats=await FeishuService.ListChatsAsync(Feishu(),client,"fixture-secret",TestToken);
        Assert.Equal(new[]{"one","two"},chats.Select(x=>x.ChatId));
    }

    [Fact]
    public async Task DingTalkUsesNumericAgentCommaRecipientsAndOfficialNotificationEndpoint()
    {
        using var handler=new FakeHandler(async(request,call,token)=>
        {
            if(call is 1 or 4)return Json("""{"errcode":0,"access_token":"fixture-token"}""");
            if(call==2)return Json("""{"errcode":0,"media_id":"fixture-media"}""");
            Assert.Equal("/topapi/message/corpconversation/asyncsend_v2",request.RequestUri!.AbsolutePath);
            var payload=JsonNode.Parse(await request.Content!.ReadAsStringAsync(token))!;
            Assert.Equal(123L,payload["agent_id"]!.GetValue<long>());
            Assert.Equal("alice,bob",payload["userid_list"]!.GetValue<string>());
            Assert.False(payload["to_all_user"]!.GetValue<bool>());
            Assert.Equal("fixture-media",payload["msg"]!["image"]!["media_id"]!.GetValue<string>());
            return Json("""{"errcode":0,"task_id":42}""");
        });
        using var client=new HttpClient(handler);
        await DingTalkService.SendImageAsync(DingTalk(),[1],client,"fixture-secret",TestToken);
        Assert.False(handler.Disposed);
        await DingTalkService.TestConnectionAsync(DingTalk(),client,"fixture-secret",TestToken);
        Assert.Equal(4,handler.Calls);
    }

    [Theory]
    [InlineData("[]",200)]
    [InlineData("{not-json",200)]
    [InlineData("{\"code\":\"0\",\"tenant_access_token\":\"private-fixture\"}",200)]
    [InlineData("{\"code\":0,\"tenant_access_token\":\"\"}",200)]
    [InlineData("{\"code\":999,\"msg\":\"private-fixture\"}",200)]
    [InlineData("{\"code\":0,\"tenant_access_token\":\"private-fixture\"}",500)]
    public async Task InvalidResponsesStopBeforeUploadAndDoNotEchoPayload(string body,int status)
    {
        using var handler=new FakeHandler((r,c,t)=>Task.FromResult(Json(body,(HttpStatusCode)status)));
        using var client=new HttpClient(handler);
        var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>FeishuService.SendImageAsync(Feishu(),[1],client,"fixture-secret",TestToken));
        Assert.DoesNotContain("private-fixture",error.ToString());
        Assert.DoesNotContain("fixture-secret",error.ToString());
        Assert.Equal(1,handler.Calls);
    }

    [Fact]
    public async Task CancellationBetweenUploadAndSendDoesNotSend()
    {
        using var cancellation=CancellationTokenSource.CreateLinkedTokenSource(TestToken);
        using var handler=new FakeHandler((r,call,t)=>
        {
            if(call==1)return Task.FromResult(Json("""{"code":0,"tenant_access_token":"fixture-token"}"""));
            cancellation.Cancel();
            return Task.FromResult(Json("""{"code":0,"data":{"image_key":"fixture-image"}}"""));
        });
        using var client=new HttpClient(handler);
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>FeishuService.SendImageAsync(Feishu(),[1],client,"fixture-secret",cancellation.Token));
        Assert.Equal(2,handler.Calls);
    }

    [Fact]
    public async Task ConcurrentAccountsDoNotReuseOneAnotherToken()
    {
        using var handler=new FakeHandler(async(r,c,t)=>
        {
            var payload=JsonNode.Parse(await r.Content!.ReadAsStringAsync(t))!;
            return Json(new JsonObject{["code"]=0,["tenant_access_token"]=payload["app_id"]!.GetValue<string>()}.ToJsonString());
        });
        using var client=new HttpClient(handler);
        var a=Feishu();var b=Feishu();a.FeishuAppId="account-a";b.FeishuAppId="account-b";
        var tokens=await Task.WhenAll(FeishuService.GetTenantTokenAsync(a,client,"fixture-a",TestToken),FeishuService.GetTenantTokenAsync(b,client,"fixture-b",TestToken));
        Assert.Equal(new[]{"account-a","account-b"},tokens);
    }

    [Fact]
    public async Task ImaUsesOfficialAddableListAndCursor()
    {
        using var handler=new FakeHandler(async(r,call,t)=>
        {
            var payload=JsonNode.Parse(await r.Content!.ReadAsStringAsync(t))!;
            Assert.Equal(call==1?"":"next-fixture",payload["cursor"]!.GetValue<string>());
            Assert.Equal(50,payload["limit"]!.GetValue<int>());
            return Json(call==1?"""{"code":0,"data":{"addable_knowledge_base_list":[{"id":"one","name":"One"}],"is_end":false,"next_cursor":"next-fixture"}}""":"""{"code":0,"data":{"addable_knowledge_base_list":[{"id":"two","name":"Two"}],"is_end":true}}""");
        });
        using var client=new HttpClient(handler);
        var bases=await ImaVaultService.GetKnowledgeBasesAsync(Ima(),client,"fixture-secret",TestToken);
        Assert.Equal(new[]{"one","two"},bases.Select(x=>x.Id));
        Assert.False(handler.Disposed);
    }

    [Fact]
    public async Task ImaUploadSignsExactObjectKeyAndUsesOnlySelectedBase()
    {
        var now=DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        var cos=Cos(now);
        const string key="folder/截图 #?%.png";
        cos["cos_key"]=key;
        using var handler=new FakeHandler(async(r,call,t)=>
        {
            if(call==2)
            {
                Assert.Equal(HttpMethod.Put,r.Method);
                Assert.Equal("fixture-123.cos.ap-guangzhou.myqcloud.com",r.RequestUri!.Host);
                Assert.Empty(r.RequestUri.Query);Assert.Empty(r.RequestUri.Fragment);
                Assert.Equal("/"+key,Uri.UnescapeDataString(r.RequestUri.AbsolutePath));
                Assert.False(r.Headers.Contains("ima-openapi-apikey"));
                Assert.Equal("fixture-sts",r.Headers.GetValues("x-cos-security-token").Single());
                var keyTime=$"{now-60};{now+600}";
                var httpString=$"put\n/{key}\n\ncontent-type=image%2Fpng&host=fixture-123.cos.ap-guangzhou.myqcloud.com\n";
                var signKey=HexHmac("fixture-cos-secret",keyTime);
                var signature=HexHmac(signKey,$"sha1\n{keyTime}\n{Convert.ToHexString(SHA1.HashData(Encoding.UTF8.GetBytes(httpString))).ToLowerInvariant()}\n");
                Assert.EndsWith("q-signature="+signature,r.Headers.GetValues("Authorization").Single());
                Assert.Equal(new byte[]{1,2,3},await r.Content!.ReadAsByteArrayAsync(t));
                return new HttpResponseMessage(HttpStatusCode.OK);
            }
            var body=JsonNode.Parse(await r.Content!.ReadAsStringAsync(t))!;
            Assert.Equal("fixture-base",body["knowledge_base_id"]!.GetValue<string>());
            Assert.Equal("fixture-secret",r.Headers.GetValues("ima-openapi-apikey").Single());
            if(call==1)return Json(new JsonObject{["code"]=0,["data"]=new JsonObject{["media_id"]="fixture-media",["cos_credential"]=cos}}.ToJsonString());
            Assert.Equal("/openapi/wiki/v1/add_knowledge",r.RequestUri!.AbsolutePath);
            Assert.Equal(9,body["media_type"]!.GetValue<int>());
            Assert.Equal(key,body["file_info"]!["cos_key"]!.GetValue<string>());
            return Json("""{"code":0,"data":{"media_id":"fixture-media"}}""");
        });
        using var client=new HttpClient(handler);
        await ImaVaultService.SaveImageAsync(Ima(),[1,2,3],client,"fixture-secret",TestToken);
        Assert.Equal(3,handler.Calls);Assert.False(handler.Disposed);
    }

    [Theory]
    [InlineData("bucket_name","evil.example/path")]
    [InlineData("region","example.com@evil.example")]
    [InlineData("cos_key","../escape")]
    [InlineData("token","")]
    [InlineData("expired_time","1")]
    public async Task ImaRejectsInvalidCosBeforeUpload(string field,string value)
    {
        var cos=Cos(DateTimeOffset.UtcNow.ToUnixTimeSeconds());cos[field]=value;
        using var handler=new FakeHandler((r,c,t)=>Task.FromResult(Json(new JsonObject{["code"]=0,["data"]=new JsonObject{["media_id"]="fixture-media",["cos_credential"]=cos}}.ToJsonString())));
        using var client=new HttpClient(handler);
        await Assert.ThrowsAsync<InvalidOperationException>(()=>ImaVaultService.SaveImageAsync(Ima(),[1],client,"fixture-secret",TestToken));
        Assert.Equal(1,handler.Calls);
    }

    [Fact]
    public async Task ImaExplicitDefaultUsesFirstWritableBase()
    {
        var settings=Ima();settings.ImaKnowledgeBaseId="";
        using var handler=new FakeHandler(async(r,c,t)=>
        {
            if(c==1)return Json("""{"code":0,"data":{"addable_knowledge_base_list":[{"id":"default-base","name":"Default"}],"is_end":true}}""");
            var payload=JsonNode.Parse(await r.Content!.ReadAsStringAsync(t))!;
            Assert.Equal("default-base",payload["knowledge_base_id"]!.GetValue<string>());
            return Json("""{"code":110030}""");
        });
        using var client=new HttpClient(handler);
        await Assert.ThrowsAsync<InvalidOperationException>(()=>ImaVaultService.SaveImageAsync(settings,[1],client,"fixture-secret",TestToken));
        Assert.Equal(2,handler.Calls);
    }

    [Fact]
    public async Task RepeatedListCursorsAreRejectedInsteadOfLooping()
    {
        using var handler=new FakeHandler((r,c,t)=>Task.FromResult(Json("""{"code":0,"data":{"addable_knowledge_base_list":[],"is_end":false,"next_cursor":"same"}}""")));
        using var client=new HttpClient(handler);
        await Assert.ThrowsAsync<InvalidOperationException>(()=>ImaVaultService.GetKnowledgeBasesAsync(Ima(),client,"fixture-secret",TestToken));
        Assert.Equal(2,handler.Calls);
    }

    [Fact]
    public async Task ImaDoesNotFallbackWhenExplicitKnowledgeBaseFails()
    {
        using var handler=new FakeHandler((r,c,t)=>
        {
            Assert.Equal("/openapi/wiki/v1/create_media",r.RequestUri!.AbsolutePath);
            return Task.FromResult(Json("""{"code":110030,"msg":"private-fixture"}"""));
        });
        using var client=new HttpClient(handler);
        await Assert.ThrowsAsync<InvalidOperationException>(()=>ImaVaultService.SaveImageAsync(Ima(),[1],client,"fixture-secret",TestToken));
        Assert.Equal(1,handler.Calls);
    }

    private static JsonObject Cos(long now)=>new(){["secret_id"]="fixture-ak",["secret_key"]="fixture-cos-secret",["token"]="fixture-sts",["bucket_name"]="fixture-123",["region"]="ap-guangzhou",["cos_key"]="folder/image.png",["start_time"]=now-60,["expired_time"]=now+600};
    private static string HexHmac(string key,string input)=>Convert.ToHexString(HMACSHA1.HashData(Encoding.UTF8.GetBytes(key),Encoding.UTF8.GetBytes(input))).ToLowerInvariant();
    private static HttpResponseMessage Json(string body,HttpStatusCode status=HttpStatusCode.OK)=>new(status){Content=new StringContent(body,Encoding.UTF8,"application/json")};

    private sealed class FakeHandler(Func<HttpRequestMessage,int,CancellationToken,Task<HttpResponseMessage>> send):HttpMessageHandler
    {
        internal int Calls;
        internal bool Disposed;
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            return send(request,Interlocked.Increment(ref Calls),cancellationToken);
        }
        protected override void Dispose(bool disposing){Disposed=true;base.Dispose(disposing);}
    }
}
