// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Text;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ProviderHttpErrorTests
{
    [Theory]
    [InlineData("<html>secret</html>")]
    [InlineData("{\"error\":")]
    [InlineData("null")]
    [InlineData("[]")]
    [InlineData("{\"detail\":[null,5,{}],\"error\":\"secret\"}")]
    public void MalformedAndUnknownErrorsKeepStatusWithoutEchoingBody(string body)
    {
        var error=ProviderHttpError.Create(422,true,Encoding.UTF8.GetBytes(body));
        Assert.Contains("HTTP 422",error.Message);
        Assert.DoesNotContain("secret",error.Message);
    }

    [Fact]
    public void ValidationReportsKnownFieldWithoutEchoingInputOrArbitraryLocation()
    {
        var error=ProviderHttpError.Create(422,true,Encoding.UTF8.GetBytes("""
            {"detail":[{"loc":["body","messages",0,"content",1,"video_url","fps","secret-path"],
              "msg":"secret-key input invalid","input":"data:video/mp4;base64,secret-media"}]}
            """));
        Assert.Contains("fps",error.Message);
        Assert.DoesNotContain("secret",error.Message);
        Assert.DoesNotContain("base64",error.Message);
    }

    [Fact]
    public void DecodeFailureUsesFixedTranslationAndNumericServiceCode()
    {
        var error=ProviderHttpError.Create(422,true,Encoding.UTF8.GetBytes("""
            {"base_resp":{"status_code":2013,"status_msg":"failed to decode video data:video/mp4;base64,secret-media"}}
            """));
        Assert.Contains("H.264",error.Message);
        Assert.Contains("2013",error.Message);
        Assert.DoesNotContain("secret",error.Message);
    }

    [Fact]
    public void VideoContentRejectionUsesContentGuidanceWithoutEchoingProviderText()
    {
        var error=ProviderHttpError.Create(422,true,Encoding.UTF8.GetBytes("""
            {"error":{"type":"invalid_request_error","message":"video content was rejected by the service"}}
            """));
        Assert.Contains(LocalizationService.T("视频内容","video's content"),error.Message);
        Assert.Contains(LocalizationService.T("其他可用 AI 渠道","another available AI channel"),error.Message);
        Assert.DoesNotContain("rejected by the service",error.Message);
    }

    [Theory]
    [InlineData("1039","视频和历史内容","video and history")]
    [InlineData("2013","请求参数","request parameters")]
    public void KnownMiniMaxCodesHaveSafeActionableGuidance(string code,string expectedChinese,string expectedEnglish)
    {
        var error=ProviderHttpError.Create(422,true,Encoding.UTF8.GetBytes("{\"base_resp\":{\"status_code\":"+code+"}}"));
        Assert.Contains(LocalizationService.T(expectedChinese,expectedEnglish),error.Message);
        Assert.Contains(code,error.Message);
    }

    [Theory]
    [InlineData("trace_id")]
    [InlineData("Trace-Id")]
    [InlineData("X-Mm-Request-Id")]
    public async Task SafeTraceIdIsShownButUnsafeHeaderIsNot(string header)
    {
        using var response=new HttpResponseMessage((HttpStatusCode)422){Content=new StringContent("{}")};
        response.Headers.Add(header,"trace_2026-09-07");
        response.Headers.Add("x-request-id","secret value");
        var error=await ProviderHttpError.ReadAsync(response,true,TestContext.Current.CancellationToken);
        Assert.Contains("trace_2026-09-07",error.Message);
        Assert.DoesNotContain("secret",error.Message);
    }

    [Fact]
    public async Task UnknownLengthResponseReadsAtMostSixteenKiBPlusOne()
    {
        using var stream=new InfiniteErrorStream();
        using var response=new HttpResponseMessage((HttpStatusCode)422){Content=new StreamContent(stream)};
        var error=await ProviderHttpError.ReadAsync(response,true,TestContext.Current.CancellationToken);
        Assert.Equal(16*1024+1,stream.BytesRead);
        Assert.Contains("HTTP 422",error.Message);
    }

    [Fact]
    public async Task CallerCancellationIsNotConvertedIntoHttpFailure()
    {
        using var cancelled=new CancellationTokenSource();cancelled.Cancel();
        using var response=new HttpResponseMessage((HttpStatusCode)422){Content=new StringContent(new string('x',20000))};
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>ProviderHttpError.ReadAsync(response,true,cancelled.Token));
    }

    [Fact]
    public async Task ProviderSurfacesSafeReasonAndClearsOwnedAttachmentOnHttpFailure()
    {
        var bytes=new byte[]{1,2,3};
        var provider=new OpenAiCompatibleProvider(
            new AiProviderSettings{Type="OpenAICompatible",BaseUrl="https://example.invalid/v1",Model="gpt-4o"},"unused",
            (_,_,_)=>Task.FromResult(new HttpResponseMessage((HttpStatusCode)422)
            {Content=new StringContent("{\"error\":{\"message\":\"invalid base64 secret\",\"param\":\"image_url\"}}")}),
            _=>TimeSpan.FromMinutes(1));
        var error=await Assert.ThrowsAsync<InvalidOperationException>(()=>provider.SendAsync(new AiRequest
        {Prompt="test",Attachments=[new(AiAttachmentType.Image,"image/png",bytes)]},TestContext.Current.CancellationToken));
        Assert.Contains("HTTP 422",error.Message);
        Assert.Contains("image_url",error.Message);
        Assert.DoesNotContain("secret",error.Message);
        Assert.All(bytes,value=>Assert.Equal(0,value));
    }

    private sealed class InfiniteErrorStream:Stream
    {
        public int BytesRead {get;private set;}
        public override bool CanRead=>true;
        public override bool CanSeek=>false;
        public override bool CanWrite=>false;
        public override long Length=>throw new NotSupportedException();
        public override long Position {get=>throw new NotSupportedException();set=>throw new NotSupportedException();}
        public override int Read(byte[] buffer,int offset,int count){Array.Fill(buffer,(byte)'x',offset,count);BytesRead+=count;return count;}
        public override ValueTask<int> ReadAsync(Memory<byte> buffer,CancellationToken cancellationToken=default)
        {cancellationToken.ThrowIfCancellationRequested();buffer.Span.Fill((byte)'x');BytesRead+=buffer.Length;return ValueTask.FromResult(buffer.Length);}
        public override void Flush()=>throw new NotSupportedException();
        public override long Seek(long offset,SeekOrigin origin)=>throw new NotSupportedException();
        public override void SetLength(long value)=>throw new NotSupportedException();
        public override void Write(byte[] buffer,int offset,int count)=>throw new NotSupportedException();
    }
}
