// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class QqMailProtocolAuditTests
{
    [Fact]
    public async Task SseSeparatesNotificationsAndMultilineResponseWithoutWaitingForEof()
    {
        const string events="""
            : keepalive

            data: {"jsonrpc":"2.0","method":"notifications/progress","params":{"progress":1}}

            event: message
            data: {"jsonrpc":"2.0","id":42,
            data: "result":{"content":[{"type":"text","text":"synthetic"}]}}


            """;
        using var timeout=new CancellationTokenSource(TimeSpan.FromSeconds(3));
        using var stream=new PersistentStream(Encoding.UTF8.GetBytes(events));
        using var response=new HttpResponseMessage(HttpStatusCode.OK){Content=new StreamContent(stream)};
        response.Content.Headers.ContentType=new("text/event-stream");
        var result=await QqMailMcpService.ReadRpcResultAsync(response,42,timeout.Token);
        Assert.Equal("synthetic",QqMailMcpService.GetTextContent(result));
        Assert.False(stream.ReadPastEnd);
    }

    [Theory]
    [InlineData("{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}")]
    [InlineData("{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"result\":{}}")]
    [InlineData("{\"id\":1,\"result\":{}}")]
    [InlineData("{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{},\"error\":{}}")]
    [InlineData("{\"jsonrpc\":\"2.0\",\"id\":1}")]
    [InlineData("[]")]
    public void RejectsUncorrelatedOrAmbiguousEnvelope(string json)
        =>Assert.Throws<InvalidDataException>(()=>QqMailMcpService.ParseRpcResult(json,1));

    [Fact]
    public async Task PlainJsonAndMultipleTextBlocksArePreserved()
    {
        using var response=new HttpResponseMessage(HttpStatusCode.OK)
        {
            Content=new StringContent("""{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"first"},{"type":"image","data":"ignored"},{"type":"text","text":"第二段"}]}}""",Encoding.UTF8,"application/json")
        };
        var result=await QqMailMcpService.ReadRpcResultAsync(response,1,CancellationToken.None);
        Assert.Equal("first\n第二段",QqMailMcpService.GetTextContent(result));
    }

    [Fact]
    public async Task NotificationsAloneAreNotASuccessfulResponse()
    {
        using var response=new HttpResponseMessage(HttpStatusCode.OK)
        {Content=new StringContent("data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n",Encoding.UTF8,"text/event-stream")};
        await Assert.ThrowsAsync<InvalidDataException>(()=>QqMailMcpService.ReadRpcResultAsync(response,1,CancellationToken.None));
    }

    [Theory]
    [InlineData("GET","/callback","expected",true)]
    [InlineData("GET","/favicon.ico","expected",false)]
    [InlineData("POST","/callback","expected",false)]
    [InlineData("GET","/callback","wrong",false)]
    [InlineData("GET","/callback",null,false)]
    public void CallbackRequiresPathMethodAndBoundState(string method,string path,string? state,bool expected)
        =>Assert.Equal(expected,QqMailMcpService.IsExpectedCallback(method,path,state,"expected"));

    [Fact]
    public void RegistrationUsesTheProductIdentity()=>Assert.Equal("MewuAI",QqMailMcpService.RegisterClientName);

    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public async Task EachCallInitializesAndCarriesOnlyItsOwnSession(bool stateful)
    {
        var calls=new List<string>();var seenSessions=new List<string?>();
        using var client=new HttpClient(new MockHandler(async(request,ct)=>
        {
            var message=JsonNode.Parse(await request.Content!.ReadAsStringAsync(ct))!.AsObject();
            var method=message["method"]!.GetValue<string>();calls.Add(method);
            var session=request.Headers.TryGetValues("Mcp-Session-Id",out var values)?Assert.Single(values):null;
            seenSessions.Add(session);
            if(method=="initialize")
            {
                Assert.Null(session);Assert.False(request.Headers.Contains("MCP-Protocol-Version"));
                Assert.Equal("MewuAI",message["params"]!["clientInfo"]!["name"]!.GetValue<string>());
                var reply=JsonReply(message["id"]!,new JsonObject{["protocolVersion"]="2025-06-18",["capabilities"]=new JsonObject{["tools"]=new JsonObject()}});
                if(stateful)reply.Headers.Add("Mcp-Session-Id","synthetic-session");return reply;
            }
            Assert.Equal("2025-06-18",Assert.Single(request.Headers.GetValues("MCP-Protocol-Version")));
            Assert.Equal(stateful?"synthetic-session":null,session);
            if(method=="notifications/initialized")
            {
                Assert.False(message.ContainsKey("id"));return new HttpResponseMessage(HttpStatusCode.Accepted);
            }
            return JsonReply(message["id"]!,new JsonObject{["content"]=new JsonArray()});
        }));
        var token=new QqMailMcpToken("synthetic-token","","synthetic-client",DateTimeOffset.UtcNow.AddHours(1),"mail:send");
        await QqMailMcpService.CallWithClientAsync(client,token,"tools/call",new JsonObject{["name"]="SendMessage"},CancellationToken.None);
        Assert.Equal(new[]{"initialize","notifications/initialized","tools/call"},calls);
        // The borrowed HttpClient remains usable, but initialization runs anew.
        await QqMailMcpService.CallWithClientAsync(client,token with{AccessToken="second-synthetic-token"},"tools/list",null,CancellationToken.None);
        Assert.Equal("initialize",calls[3]);Assert.Null(seenSessions[3]);
    }

    [Fact]
    public async Task FailedMutatingRequestIsNeverReplayed()
    {
        var mutatingCalls=0;
        using var client=new HttpClient(new MockHandler(async(request,ct)=>
        {
            var node=JsonNode.Parse(await request.Content!.ReadAsStringAsync(ct))!.AsObject();
            return node["method"]!.GetValue<string>() switch
            {
                "initialize"=>JsonReply(node["id"]!,new JsonObject{["protocolVersion"]="2025-11-25",["capabilities"]=new JsonObject{["tools"]=new JsonObject()}}),
                "notifications/initialized"=>new HttpResponseMessage(HttpStatusCode.Accepted),
                _=>Fail()
            };
            HttpResponseMessage Fail(){mutatingCalls++;return new(HttpStatusCode.ServiceUnavailable){Content=new StringContent("synthetic-private-body")};}
        }));
        var token=new QqMailMcpToken("synthetic-token","","synthetic-client",DateTimeOffset.UtcNow.AddHours(1),"mail:send");
        var error=await Assert.ThrowsAsync<HttpRequestException>(()=>QqMailMcpService.CallWithClientAsync(client,token,"tools/call",new JsonObject{["name"]="SendMessage"},CancellationToken.None));
        Assert.Equal(1,mutatingCalls);Assert.DoesNotContain("synthetic-private-body",error.ToString());
    }

    [Fact]
    public void RpcErrorsNeverEchoMailboxOrTokenDetails()
    {
        var error=Assert.Throws<InvalidOperationException>(()=>QqMailMcpService.ParseRpcResult("""{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"synthetic-private-body","data":{"sid":"synthetic-private-token"}}}""",1));
        Assert.DoesNotContain("synthetic-private",error.ToString());
    }

    [Fact]
    public void InvalidJsonDoesNotExposePropertyNamesInLoggedExceptions()
    {
        var error=Assert.Throws<InvalidDataException>(()=>QqMailMcpService.ParseRpcResult("{\"synthetic-private-token\": invalid}",1));
        Assert.DoesNotContain("synthetic-private",error.ToString());Assert.Null(error.InnerException);
    }

    [Fact]
    public async Task InitializationFailureHasNoToolDispatchAndRemainsDefinite()
    {
        var calls=0;
        using var client=new HttpClient(new MockHandler((_,_)=>
        {calls++;return Task.FromException<HttpResponseMessage>(new IOException("synthetic initialization disconnect"));}));
        var token=new QqMailMcpToken("synthetic","","client",DateTimeOffset.UtcNow.AddHours(1),"mail:send");
        await Assert.ThrowsAsync<InvalidOperationException>(()=>QqMailSendService.DeliverQqAsync(
            new QqMailDraft([("recipient@example.test",null)],[],"Subject","Body"),_=>Task.FromResult(true),async(args,ct)=>
            {var result=await QqMailMcpService.CallWithClientAsync(client,token,"tools/call",args,ct);return(QqMailMcpService.GetTextContent(result),false);},TestContext.Current.CancellationToken));
        Assert.Equal(1,calls);
    }

    [Theory]
    [InlineData("malformed JSON")]
    [InlineData("{\"jsonrpc\":\"2.0\",\"id\":-999,\"result\":{}}")]
    public async Task InvalidReplyAfterToolDispatchIsUnconfirmedWithoutReplay(string reply)
    {
        var toolCalls=0;
        using var client=new HttpClient(new MockHandler(async(request,ct)=>
        {
            var node=JsonNode.Parse(await request.Content!.ReadAsStringAsync(ct))!.AsObject();
            if(node["method"]!.GetValue<string>()=="initialize")return JsonReply(node["id"]!,new JsonObject{["protocolVersion"]="2025-11-25",["capabilities"]=new JsonObject{["tools"]=new JsonObject()}});
            if(node["method"]!.GetValue<string>()=="notifications/initialized")return new HttpResponseMessage(HttpStatusCode.Accepted);
            toolCalls++;return new HttpResponseMessage(HttpStatusCode.OK){Content=new StringContent(reply)};
        }));
        var token=new QqMailMcpToken("synthetic","","client",DateTimeOffset.UtcNow.AddHours(1),"mail:send");
        var outcome=await QqMailSendService.DeliverQqAsync(new QqMailDraft([("recipient@example.test",null)],[],"Subject","Body"),
            _=>Task.FromResult(true),async(args,ct)=>
            {var result=await QqMailMcpService.CallWithClientAsync(client,token,"tools/call",args,ct);return(QqMailMcpService.GetTextContent(result),false);},TestContext.Current.CancellationToken);
        Assert.True(outcome.IsUnconfirmed);Assert.False(outcome.Success);Assert.Equal(1,toolCalls);
    }

    private static HttpResponseMessage JsonReply(JsonNode id,JsonObject result)
        =>new(HttpStatusCode.OK){Content=new StringContent(new JsonObject{["jsonrpc"]="2.0",["id"]=id.DeepClone(),["result"]=result}.ToJsonString(),Encoding.UTF8,"application/json")};

    private sealed class MockHandler(Func<HttpRequestMessage,CancellationToken,Task<HttpResponseMessage>> send):HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken cancellationToken)=>send(request,cancellationToken);
    }

    private sealed class PersistentStream(byte[] bytes):Stream
    {
        private int _position;
        public bool ReadPastEnd{get;private set;}
        public override bool CanRead=>true;
        public override bool CanSeek=>false;
        public override bool CanWrite=>false;
        public override long Length=>throw new NotSupportedException();
        public override long Position{get=>_position;set=>throw new NotSupportedException();}
        public override async ValueTask<int> ReadAsync(Memory<byte> buffer,CancellationToken cancellationToken=default)
        {
            if(_position==bytes.Length){ReadPastEnd=true;await Task.Delay(Timeout.Infinite,cancellationToken);return 0;}
            var count=Math.Min(buffer.Length,bytes.Length-_position);bytes.AsMemory(_position,count).CopyTo(buffer);_position+=count;return count;
        }
        public override int Read(byte[] buffer,int offset,int count)=>throw new NotSupportedException();
        public override void Flush(){}
        public override long Seek(long offset,SeekOrigin origin)=>throw new NotSupportedException();
        public override void SetLength(long value)=>throw new NotSupportedException();
        public override void Write(byte[] buffer,int offset,int count)=>throw new NotSupportedException();
    }
}
