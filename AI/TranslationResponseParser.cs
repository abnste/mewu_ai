// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace mewu_ai_Assistant.AI;

/// <summary>Reads the small, ordered JSON payload used by in-place translation.</summary>
public static class TranslationResponseParser
{
    private const int MaxPayloadCharacters=256*1024;
    private static readonly string[] TranslationPropertyNames=["translations","translation","translatedText","translated_text"];
    private static readonly string[] WrapperPropertyNames=["data","result","output"];
    private static readonly string[] TranslationValueNames=["translation","translatedText","translated_text","text"];

    public static bool TryParse(string? value,int expectedCount,out IReadOnlyList<string> translations)=>
        TryParseCore(value,expectedCount,_=>false,out translations);

    /// <summary>
    /// Preserves a blank OCR line as a blank translation, while still rejecting
    /// an omitted translation for every visible source line.
    /// </summary>
    public static bool TryParse(string? value,IReadOnlyList<string> sourceLines,out IReadOnlyList<string> translations)
    {
        ArgumentNullException.ThrowIfNull(sourceLines);
        return TryParseCore(value,sourceLines.Count,index=>string.IsNullOrWhiteSpace(sourceLines[index]),out translations);
    }

    private static bool TryParseCore(string? value,int expectedCount,Func<int,bool> allowsEmpty,out IReadOnlyList<string> translations)
    {
        translations=[];
        if(expectedCount<0||string.IsNullOrWhiteSpace(value)||value.Length>MaxPayloadCharacters)return false;
        var payload=RemoveMarkdownFence(value);
        if(TryParseFragments(payload,expectedCount,allowsEmpty,out translations))return true;
        if(TryReadQuotedPayload(payload,out var quoted)&&TryParseFragments(quoted,expectedCount,allowsEmpty,out translations))return true;
        return false;
    }

    private static bool TryParseFragments(string payload,int expectedCount,Func<int,bool> allowsEmpty,out IReadOnlyList<string> translations)
    {
        translations=[];
        if(payload.Length==0||payload.Length>MaxPayloadCharacters)return false;
        var utf8=Encoding.UTF8.GetBytes(payload);
        try
        {
            for(var offset=0;offset<utf8.Length;offset++)
            {
                if(utf8[offset] is not ((byte)'{') and not ((byte)'['))continue;
                try
                {
                    var reader=new Utf8JsonReader(utf8.AsSpan(offset),isFinalBlock:true,state:default);
                    using var document=JsonDocument.ParseValue(ref reader);
                    if(TryReadTranslations(document.RootElement,expectedCount,allowsEmpty,0,out translations))return true;
                }
                catch(JsonException)
                {
                    // A response may contain prose, a first unrelated JSON
                    // value, or a partially streamed candidate before the
                    // final object. Continue looking for the translation root.
                }
            }
            return false;
        }
        finally{CryptographicOperations.ZeroMemory(utf8);}
    }

    private static bool TryReadQuotedPayload(string payload,out string quoted)
    {
        quoted=string.Empty;
        var trimmed=payload.TrimStart();
        if(!trimmed.StartsWith('"'))return false;
        try
        {
            using var document=JsonDocument.Parse(trimmed);
            if(document.RootElement.ValueKind!=JsonValueKind.String)return false;
            quoted=document.RootElement.GetString()??string.Empty;
            return quoted.Length>0&&quoted.Length<=MaxPayloadCharacters;
        }
        catch(JsonException){return false;}
    }

    private static bool TryReadTranslations(JsonElement root,int expectedCount,Func<int,bool> allowsEmpty,int wrapperDepth,out IReadOnlyList<string> translations)
    {
        translations=[];
        if(root.ValueKind==JsonValueKind.Object)
        {
            if(TryGetProperty(root,TranslationPropertyNames,out var value))root=value;
            else if(wrapperDepth==0&&TryGetProperty(root,WrapperPropertyNames,out var wrapped))return TryReadTranslations(wrapped,expectedCount,allowsEmpty,wrapperDepth+1,out translations);
            else return false;
        }
        if(root.ValueKind!=JsonValueKind.Array)return false;
        var values=new List<string>();
        foreach(var item in root.EnumerateArray())
        {
            if(item.ValueKind==JsonValueKind.String)values.Add(item.GetString()??string.Empty);
            else if(item.ValueKind==JsonValueKind.Object&&TryGetProperty(item,TranslationValueNames,out var translated)&&translated.ValueKind==JsonValueKind.String)values.Add(translated.GetString()??string.Empty);
            else return false;
        }
        if(values.Count!=expectedCount)return false;
        for(var index=0;index<values.Count;index++)
            if(string.IsNullOrWhiteSpace(values[index])&&!allowsEmpty(index))return false;
        translations=values;
        return true;
    }

    private static bool TryGetProperty(JsonElement element,IReadOnlyList<string> names,out JsonElement value)
    {
        if(element.ValueKind==JsonValueKind.Object)
            foreach(var property in element.EnumerateObject())
                if(names.Any(name=>name.Equals(property.Name,StringComparison.OrdinalIgnoreCase)))
                {
                    value=property.Value;
                    return true;
                }
        value=default;
        return false;
    }

    private static string RemoveMarkdownFence(string value)
    {
        var trimmed=value.Trim().TrimStart('\uFEFF');
        if(!trimmed.StartsWith("```",StringComparison.Ordinal))return trimmed;
        var firstLineEnd=trimmed.IndexOf('\n');
        if(firstLineEnd<0)return trimmed;
        var body=trimmed[(firstLineEnd+1)..];
        var closing=body.LastIndexOf("```",StringComparison.Ordinal);
        return (closing>=0?body[..closing]:body).Trim();
    }
}
