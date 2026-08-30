using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace mewu_ai_Assistant.AI;

public static class TranslationResponseParser
{
    public static bool TryParse(string value,int expectedCount,out IReadOnlyList<string> translations)
    {
        translations=[];
        if(expectedCount<0)return false;
        var payload=RemoveMarkdownFence(value);
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
                    if(TryReadTranslations(document.RootElement,expectedCount,out translations))return true;
                }
                catch(JsonException)
                {
                }
            }
            return false;
        }
        finally{CryptographicOperations.ZeroMemory(utf8);}
    }

    private static bool TryReadTranslations(JsonElement root,int expectedCount,out IReadOnlyList<string> translations)
    {
        translations=[];
        if(root.ValueKind==JsonValueKind.Object)
        {
            var property=root.EnumerateObject().FirstOrDefault(x=>x.Name.Equals("translations",StringComparison.OrdinalIgnoreCase));
            if(property.Value.ValueKind==JsonValueKind.Undefined)return false;
            root=property.Value;
        }
        if(root.ValueKind!=JsonValueKind.Array)return false;
        var values=root.EnumerateArray().Select(item=>item.ValueKind==JsonValueKind.String?item.GetString()??string.Empty:string.Empty).ToList();
        if(values.Count!=expectedCount||values.Any(string.IsNullOrWhiteSpace))return false;
        translations=values;
        return true;
    }

    private static string RemoveMarkdownFence(string value)
    {
        var trimmed=value.Trim();
        if(!trimmed.StartsWith("```",StringComparison.Ordinal))return trimmed;
        var firstLineEnd=trimmed.IndexOf('\n');
        if(firstLineEnd<0)return trimmed;
        var body=trimmed[(firstLineEnd+1)..];
        var closing=body.LastIndexOf("```",StringComparison.Ordinal);
        return (closing>=0?body[..closing]:body).Trim();
    }
}
