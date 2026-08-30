using System.Text.Json;
using System.Text.RegularExpressions;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.AI;
public static class StructuredResponseParser
{
    private static readonly Regex ThinkBlock=new("<(?<tag>think|thinking|reasoning)>\\s*(?<body>[\\s\\S]*?)\\s*</\\k<tag>>",RegexOptions.IgnoreCase|RegexOptions.Compiled);
    public static AiResult Parse(string value,string reasoning="")
    {
        var extracted=ThinkBlock.Matches(value).Select(x=>x.Groups["body"].Value.Trim()).Where(x=>x.Length>0);var allReasoning=string.Join(Environment.NewLine+Environment.NewLine,new[]{reasoning.Trim()}.Concat(extracted).Where(x=>x.Length>0));value=ThinkBlock.Replace(value,string.Empty).Trim();
        var json=Regex.Match(value,"\\{[\\s\\S]*\\}").Value;if(string.IsNullOrWhiteSpace(json))return new(value,[],allReasoning);
        try{var parsed=JsonSerializer.Deserialize<StructuredAiResponse>(json);if(parsed is null||string.IsNullOrWhiteSpace(parsed.Answer))return new(value,[],allReasoning);var notes=parsed.Annotations.Where(a=>a.X>=0&&a.Y>=0&&a.Width>=0&&a.Height>=0&&a.X+a.Width<=1.001&&a.Y+a.Height<=1.001).Select(a=>new AiAnnotation(a.X,a.Y,a.Width,a.Height,a.Text,a.Type,a.RegionIndex)).ToList();return new(parsed.Answer,notes,allReasoning);}catch(JsonException){return new(value,[],allReasoning);}
    }
}
