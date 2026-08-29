using System.Text.Json;
using System.Text.RegularExpressions;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.AI;
public static class StructuredResponseParser
{
    public static AiResult Parse(string value)
    {
        var json=Regex.Match(value,"\\{[\\s\\S]*\\}").Value;if(string.IsNullOrWhiteSpace(json))return new(value,[]);
        try{var parsed=JsonSerializer.Deserialize<StructuredAiResponse>(json);if(parsed is null||string.IsNullOrWhiteSpace(parsed.Answer))return new(value,[]);var notes=parsed.Annotations.Where(a=>a.X>=0&&a.Y>=0&&a.Width>=0&&a.Height>=0&&a.X+a.Width<=1.001&&a.Y+a.Height<=1.001).Select(a=>new AiAnnotation(a.X,a.Y,a.Width,a.Height,a.Text,a.Type)).ToList();return new(parsed.Answer,notes);}catch(JsonException){return new(value,[]);}
    }
}
