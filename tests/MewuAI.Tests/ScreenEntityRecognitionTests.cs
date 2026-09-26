// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class ScreenEntityRecognitionTests
{
    [Fact]
    public void ExtractsUrlsAndProviderEmailsWithoutTrailingPunctuation()
    {
        var entities=ScreenEntityRecognitionService.Extract("访问 https://example.com/path?q=1。联系 a@qq.com、b@163.com.");
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Url&&entity.Value=="https://example.com/path?q=1");
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Email&&entity.Value=="a@qq.com"&&entity.MailProvider=="qq");
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Email&&entity.Value=="b@163.com"&&entity.MailProvider=="netease");
    }

    [Fact]
    public void IgnoresUnsupportedSchemesAndDeduplicates()
    {
        var entities=ScreenEntityRecognitionService.Extract("ftp://example.com https://example.com https://example.com x@example.org x@example.org");
        Assert.Equal(2,entities.Count);
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Url);
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Email&&entity.MailProvider is null);
    }

    [Fact]
    public void ExtractsMainlandPhoneNumbersWithOptionalCountryCodeAndSeparators()
    {
        var entities=ScreenEntityRecognitionService.Extract("联系电话：138-0013-8000，备用 +86 139 1234 5678");
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Phone&&entity.Value=="13800138000");
        Assert.Contains(entities,entity=>entity.Type==ScreenEntityType.Phone&&entity.Value=="+8613912345678");
    }

    [Fact]
    public void PhoneActionsNormalizeFormattedNumbers()
    {
        var candidate=PhoneNumberService.Classify("+86 139 1234 5678");
        Assert.NotNull(candidate);
        Assert.True(candidate!.Dialable);
        Assert.Equal("+8613912345678",candidate.Display);
    }

    [Theory]
    [InlineData(MailChannel.Auto,true,true,MailChannel.Qq)]
    [InlineData(MailChannel.Auto,false,true,MailChannel.NetEase)]
    [InlineData(MailChannel.Qq,false,true,null)]
    [InlineData(MailChannel.NetEase,true,false,null)]
    internal void MailChannelSelectionDoesNotInferSenderFromRecipient(MailChannel requested,bool qqReady,bool netEaseReady,MailChannel? expected)
        =>Assert.Equal(expected,QqMailSendService.SelectChannel(requested,qqReady,netEaseReady));
}
