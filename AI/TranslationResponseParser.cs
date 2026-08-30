using System.Text.Json;

namespace mewu_ai_Assistant.AI;

public static class TranslationResponseParser
{
    public static bool TryParse(string value,int expectedCount,out IReadOnlyList<string> translations)
    {
        translations=[];
        var payload=RemoveMarkdownFence(value);
        try
        {
            using var document=JsonDocument.Parse(payload);
            var root=document.RootElement;
            if(root.ValueKind==JsonValueKind.Object&&root.TryGetProperty("translations",out var property))root=property;
            if(root.ValueKind!=JsonValueKind.Array)return false;
            var values=root.EnumerateArray().Select(item=>item.ValueKind==JsonValueKind.String?item.GetString()??string.Empty:string.Empty).ToList();
            if(values.Count!=expectedCount||values.Any(string.IsNullOrWhiteSpace))return false;
            translations=values;
            return true;
        }
        catch(JsonException){return false;}
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
