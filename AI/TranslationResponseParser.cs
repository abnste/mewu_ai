using System.Text.Json;

namespace mewu_ai_Assistant.AI;

public static class TranslationResponseParser
{
    public static bool TryParse(string value,int expectedCount,out IReadOnlyList<string> translations)
    {
        translations=[];
        var payload=ExtractJson(RemoveMarkdownFence(value));
        try
        {
            using var document=JsonDocument.Parse(payload);
            var root=document.RootElement;
            if(root.ValueKind==JsonValueKind.Object)
            {
                var property=root.EnumerateObject().FirstOrDefault(x=>x.Name.Equals("translations",StringComparison.OrdinalIgnoreCase));
                if(property.Value.ValueKind!=JsonValueKind.Undefined)root=property.Value;
            }
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

    private static string ExtractJson(string value)
    {
        var objectStart=value.IndexOf('{');var arrayStart=value.IndexOf('[');var start=objectStart<0?arrayStart:arrayStart<0?objectStart:Math.Min(objectStart,arrayStart);if(start<0)return value;
        var end=value.LastIndexOf(value[start]=='{'?'}':']');return end>=start?value[start..(end+1)]:value;
    }
}
