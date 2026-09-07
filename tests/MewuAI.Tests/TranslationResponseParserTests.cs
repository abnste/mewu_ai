// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.AI;
using Xunit;

namespace MewuAI.Tests;

public sealed class TranslationResponseParserTests
{
    [Fact] public void ParsesObjectAndPreservesLineOrder(){Assert.True(TranslationResponseParser.TryParse("{\"translations\":[\"第一行\",\"第二行\"]}",2,out var values));Assert.Equal(["第一行","第二行"],values);}
    [Fact] public void ParsesMarkdownFencedJson(){Assert.True(TranslationResponseParser.TryParse("```json\n{\"translations\":[\"译文\"]}\n```",1,out var values));Assert.Equal("译文",Assert.Single(values));}
    [Fact] public void RejectsWrongLineCount(){Assert.False(TranslationResponseParser.TryParse("{\"translations\":[\"只有一行\"]}",2,out _));}
    [Fact] public void ParsesJsonSurroundedByProviderProse(){Assert.True(TranslationResponseParser.TryParse("结果如下： {\"Translations\":[\"译文\"]} 完成",1,out var values));Assert.Equal("译文",Assert.Single(values));}
    [Fact] public void SkipsEarlierUnrelatedJsonAndParsesTranslationsObject(){Assert.True(TranslationResponseParser.TryParse("说明 []，结果：{\"translations\":[\"译文\"]} 完成",1,out var values));Assert.Equal("译文",Assert.Single(values));}
    [Fact] public void ParsesWrappedObjectItemsAndDoubleEncodedPayload(){Assert.True(TranslationResponseParser.TryParse("{\"result\":{\"translations\":[{\"translation\":\"译文\"}]}}",1,out var wrapped));Assert.Equal("译文",Assert.Single(wrapped));Assert.True(TranslationResponseParser.TryParse("\"{\\\"translations\\\":[\\\"再次译文\\\"]}\"",1,out var encoded));Assert.Equal("再次译文",Assert.Single(encoded));}
    [Fact] public void SourceLinesAllowOnlyMatchingBlankTranslations(){Assert.True(TranslationResponseParser.TryParse("{\"translations\":[\"\",\"译文\"]}",["","visible"],out var values));Assert.Equal(["","译文"],values);Assert.False(TranslationResponseParser.TryParse("{\"translations\":[\"\",\"\"]}",["","visible"],out _));}
    [Fact] public void RejectsMalformedJsonForTheCallerToRetry(){Assert.False(TranslationResponseParser.TryParse("{\"translations\":[\"译文\",]}",1,out _));}
    [Fact] public void RejectsNegativeExpectedCount(){Assert.False(TranslationResponseParser.TryParse("[]",-1,out _));}
}
