using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public static class StructuredResponseParser
{
    private static readonly Regex ThinkBlock = new("<(?<tag>think|thinking|reasoning)>\\s*(?<body>[\\s\\S]*?)\\s*</\\k<tag>>", RegexOptions.IgnoreCase | RegexOptions.Compiled);

    public static AiResult Parse(string value, string reasoning = "")
    {
        var extracted = ThinkBlock.Matches(value).Select(x => x.Groups["body"].Value.Trim()).Where(x => x.Length > 0);
        var allReasoning = string.Join(Environment.NewLine + Environment.NewLine, new[] { reasoning.Trim() }.Concat(extracted).Where(x => x.Length > 0));
        value = ThinkBlock.Replace(value, string.Empty).Trim();

        if (!TryGetStructuredPayload(value, out var json))
            return new(value, [], allReasoning);

        try
        {
            using var document = JsonDocument.Parse(json);
            if (document.RootElement.ValueKind != JsonValueKind.Object || !document.RootElement.TryGetProperty("answer", out _))
                return new(value, [], allReasoning);

            var parsed = JsonSerializer.Deserialize<StructuredAiResponse>(json);
            if (parsed is null || string.IsNullOrWhiteSpace(parsed.Answer))
                return new(string.Empty, [], allReasoning);

            var notes = (parsed.Annotations ?? [])
                .Where(a => a is not null && a.X >= 0 && a.Y >= 0 && a.Width >= 0 && a.Height >= 0 && a.X + a.Width <= 1.001 && a.Y + a.Height <= 1.001)
                .Select(a => new AiAnnotation(a.X, a.Y, a.Width, a.Height, a.Text, a.Type, a.RegionIndex))
                .ToList();
            return new(parsed.Answer, notes, allReasoning);
        }
        catch (JsonException)
        {
            return TryExtractAnswerFromTruncatedStructuredResponse(json, out var answer)
                ? new(answer, [], allReasoning)
                : new(value, [], allReasoning);
        }
    }

    private static bool TryGetStructuredPayload(string value, out string payload)
    {
        payload = string.Empty;
        var trimmed = value.Trim();
        if (trimmed.StartsWith('{'))
        {
            payload = trimmed;
            return true;
        }

        if (!trimmed.StartsWith("```", StringComparison.Ordinal))
            return false;

        var lineEnd = trimmed.IndexOfAny(['\r', '\n'], 3);
        if (lineEnd < 0)
            return false;

        var language = trimmed[3..lineEnd].Trim();
        if (language.Length > 0 && !language.Equals("json", StringComparison.OrdinalIgnoreCase))
            return false;

        var bodyStart = lineEnd;
        if (trimmed[bodyStart] == '\r')
            bodyStart++;
        if (bodyStart < trimmed.Length && trimmed[bodyStart] == '\n')
            bodyStart++;

        var body = trimmed[bodyStart..].Trim();
        if (body.EndsWith("```", StringComparison.Ordinal))
            body = body[..^3].TrimEnd();
        if (!body.StartsWith('{'))
            return false;

        payload = body;
        return true;
    }

    private static bool TryExtractAnswerFromTruncatedStructuredResponse(string json, out string answer)
    {
        answer = string.Empty;
        if (!TryFindRootAnswerValue(json.AsSpan(), out var answerStart))
            return false;
        if (!TryFindAnswerEndBeforeAnnotations(json.AsSpan(), answerStart, out var answerEnd))
            return false;
        if (!IsValidJsonPrefixWithEmptyAnswer(json, answerStart, answerEnd))
            return false;

        return TryDecodeJsonStringLoosely(json.AsSpan(answerStart, answerEnd - answerStart), out answer);
    }

    private static bool TryFindRootAnswerValue(ReadOnlySpan<char> json, out int answerStart)
    {
        answerStart = -1;
        var first = SkipWhitespace(json, 0);
        if (first >= json.Length || json[first] != '{')
            return false;

        var objectDepth = 0;
        var arrayDepth = 0;
        for (var index = first; index < json.Length; index++)
        {
            switch (json[index])
            {
                case '{':
                    objectDepth++;
                    break;
                case '}':
                    objectDepth--;
                    break;
                case '[':
                    arrayDepth++;
                    break;
                case ']':
                    arrayDepth--;
                    break;
                case '"':
                {
                    var stringEnd = FindJsonStringEnd(json, index + 1);
                    if (stringEnd < 0)
                        return false;

                    if (objectDepth == 1 && arrayDepth == 0)
                    {
                        var colon = SkipWhitespace(json, stringEnd + 1);
                        if (colon < json.Length && json[colon] == ':' &&
                            TryDecodeJsonStringLoosely(json[(index + 1)..stringEnd], out var propertyName) &&
                            propertyName == "answer")
                        {
                            var valueStart = SkipWhitespace(json, colon + 1);
                            if (valueStart >= json.Length || json[valueStart] != '"')
                                return false;

                            answerStart = valueStart + 1;
                            return true;
                        }
                    }

                    index = stringEnd;
                    break;
                }
            }
        }

        return false;
    }

    private static bool TryFindAnswerEndBeforeAnnotations(ReadOnlySpan<char> json, int answerStart, out int answerEnd)
    {
        answerEnd = -1;
        for (var index = answerStart; index < json.Length; index++)
        {
            if (json[index] == '\\')
            {
                index++;
                continue;
            }

            if (json[index] != '"')
                continue;

            var comma = SkipWhitespace(json, index + 1);
            if (comma >= json.Length || json[comma] != ',')
                continue;

            var propertyStart = SkipWhitespace(json, comma + 1);
            if (propertyStart >= json.Length || json[propertyStart] != '"')
                continue;

            var propertyEnd = FindJsonStringEnd(json, propertyStart + 1);
            if (propertyEnd < 0 || !TryDecodeJsonStringLoosely(json[(propertyStart + 1)..propertyEnd], out var propertyName) || propertyName != "annotations")
                continue;

            var colon = SkipWhitespace(json, propertyEnd + 1);
            if (colon >= json.Length || json[colon] != ':')
                continue;

            var annotationStart = SkipWhitespace(json, colon + 1);
            if (!LooksLikeAnnotationValuePrefix(json, annotationStart))
                continue;

            answerEnd = index;
            return true;
        }

        return false;
    }

    private static bool LooksLikeAnnotationValuePrefix(ReadOnlySpan<char> json, int start)
    {
        if (start >= json.Length)
            return true;
        if (json[start] == '[')
            return true;

        const string nullLiteral = "null";
        var remaining = json[start..];
        var prefixLength = Math.Min(remaining.Length, nullLiteral.Length);
        return prefixLength > 0 && remaining[..prefixLength].SequenceEqual(nullLiteral.AsSpan(0, prefixLength));
    }

    private static bool IsValidJsonPrefixWithEmptyAnswer(string json, int answerStart, int answerEnd)
    {
        var normalized = string.Concat(json.AsSpan(0, answerStart), json.AsSpan(answerEnd));
        var utf8 = Encoding.UTF8.GetBytes(normalized);
        var reader = new Utf8JsonReader(utf8, isFinalBlock: false, state: default);
        try
        {
            while (reader.Read())
            {
            }

            return true;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    private static int FindJsonStringEnd(ReadOnlySpan<char> value, int start)
    {
        for (var index = start; index < value.Length; index++)
        {
            if (value[index] == '\\')
            {
                index++;
                continue;
            }

            if (value[index] == '"')
                return index;
        }

        return -1;
    }

    private static bool TryDecodeJsonStringLoosely(ReadOnlySpan<char> value, out string decoded)
    {
        var builder = new StringBuilder(value.Length);
        for (var index = 0; index < value.Length; index++)
        {
            var character = value[index];
            if (character != '\\')
            {
                builder.Append(character);
                continue;
            }

            if (++index >= value.Length)
            {
                decoded = string.Empty;
                return false;
            }

            switch (value[index])
            {
                case '"': builder.Append('"'); break;
                case '\\': builder.Append('\\'); break;
                case '/': builder.Append('/'); break;
                case 'b': builder.Append('\b'); break;
                case 'f': builder.Append('\f'); break;
                case 'n': builder.Append('\n'); break;
                case 'r': builder.Append('\r'); break;
                case 't': builder.Append('\t'); break;
                case 'u':
                    if (index + 4 >= value.Length || !ushort.TryParse(value.Slice(index + 1, 4), NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture, out var codePoint))
                    {
                        decoded = string.Empty;
                        return false;
                    }

                    builder.Append((char)codePoint);
                    index += 4;
                    break;
                default:
                    decoded = string.Empty;
                    return false;
            }
        }

        decoded = builder.ToString();
        return true;
    }

    private static int SkipWhitespace(ReadOnlySpan<char> value, int start)
    {
        while (start < value.Length && char.IsWhiteSpace(value[start]))
            start++;
        return start;
    }
}
