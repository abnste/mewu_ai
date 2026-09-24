// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using System.Text;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public static class StreamingResponseParser
{
    public static bool TryParse(string line,out AiStreamDelta delta,out bool done)
        =>TryParse(line,out delta,out done,out _);

    public static bool TryParse(string line,out AiStreamDelta delta,out bool done,out bool truncated)
    {
        delta=new(string.Empty,string.Empty);done=false;truncated=false;
        var trimmed=line.Trim().TrimStart('\uFEFF','\u200B');
        string payload;
        if(trimmed.StartsWith("data",StringComparison.OrdinalIgnoreCase))
        {
            // Be liberal about SSE formatting: both `data:` and `data :`
            // occur in small OpenAI-compatible proxies.
            var colon=trimmed.IndexOf(':');
            if(colon<0)return false;
            payload=trimmed[(colon+1)..].Trim();
        }
        else if(trimmed.StartsWith('{')||trimmed.StartsWith('['))
        {
            // A few gateways advertise streaming but emit newline-delimited
            // JSON without the SSE `data:` prefix.
            payload=trimmed;
        }
        else return false;
        if(payload=="[DONE]"){done=true;return true;}
        try
        {
            using var document=JsonDocument.Parse(payload);
            // Some newer OpenAI-compatible gateways expose the Responses API
            // event envelope even when /chat/completions was requested.  It
            // has no `choices` array, so handle that envelope before treating
            // the event as malformed.  Without this branch a perfectly valid
            // response is read until EOF and then reported as an interrupted
            // stream.
            if(!document.RootElement.TryGetProperty("choices",out var choices))
                return TryParseResponsesEvent(document.RootElement,out delta,out done,out truncated);
            if(choices.ValueKind!=JsonValueKind.Array)return false;
            if(choices.GetArrayLength()==0)return true;
            var choice=choices[0];
            var hasFinish=choice.TryGetProperty("finish_reason",out var finish)&&finish.ValueKind==JsonValueKind.String&&!string.IsNullOrWhiteSpace(finish.GetString());
            var candidateDone=hasFinish;
            var candidateTruncated=candidateDone&&string.Equals(finish.GetString(),"length",StringComparison.OrdinalIgnoreCase);
            if(!choice.TryGetProperty("delta",out var value))
                { done=candidateDone;truncated=candidateTruncated;return candidateDone; }
            if(value.ValueKind!=JsonValueKind.Object)
            {
                // If the endpoint ignored `stream:true`, it can return one
                // complete chat-completion object in the stream body.
                if(choice.TryGetProperty("message",out var message)&&message.ValueKind==JsonValueKind.Object)
                {
                    var (messageContent,messageTypedReasoning)=ReadContentParts(message);
                    var messageReasoning=ReadString(message,"reasoning_content");
                    if(messageReasoning.Length==0)messageReasoning=ReadString(message,"thinking_content");
                    if(messageReasoning.Length==0)messageReasoning=ReadString(message,"reasoning");
                    if(messageReasoning.Length==0)messageReasoning=messageTypedReasoning;
                    delta=new(messageContent,messageReasoning);done=true;return true;
                }
                return false;
            }
            var (content,typedReasoning)=ReadContentParts(value);var reasoning=ReadString(value,"reasoning_content");var cumulative=false;
            if(reasoning.Length==0)reasoning=ReadString(value,"thinking_content");
            if(reasoning.Length==0)reasoning=ReadString(value,"reasoning");
            if(reasoning.Length==0)reasoning=typedReasoning;
            if(reasoning.Length==0){reasoning=ReadReasoningDetails(value);cumulative=reasoning.Length>0;}
            delta=new(content,reasoning,cumulative);done=candidateDone;truncated=candidateTruncated;return true;
        }
        catch(JsonException){return false;}
        catch(KeyNotFoundException){return false;}
        catch(InvalidOperationException){return false;}
    }

    private static bool TryParseResponsesEvent(JsonElement root,out AiStreamDelta delta,out bool done,out bool truncated)
    {
        delta=new(string.Empty,string.Empty);done=false;truncated=false;
        var type=ReadString(root,"type");
        if(type.Length==0)
        {
            // Some simple relays use {"text":"..."}, {"content":"..."}
            // or {"response":"..."} as their NDJSON stream envelope.
            var text=ReadString(root,"text");
            if(text.Length==0)text=ReadString(root,"content");
            if(text.Length==0)text=ReadString(root,"response");
            if(text.Length>0){delta=new(text,string.Empty);return true;}
            if(root.TryGetProperty("done",out var finished)&&finished.ValueKind==JsonValueKind.True){done=true;return true;}
            return false;
        }

        // OpenAI Responses API text/reasoning deltas.
        if(type.Equals("response.output_text.delta",StringComparison.OrdinalIgnoreCase))
        {
            var text=ReadString(root,"delta");
            if(text.Length==0&&root.TryGetProperty("delta",out var value)&&value.ValueKind==JsonValueKind.Object)
                text=ReadString(value,"text");
            delta=new(text,string.Empty);return true;
        }
        if(type.Contains("reasoning",StringComparison.OrdinalIgnoreCase)&&type.EndsWith(".delta",StringComparison.OrdinalIgnoreCase))
        {
            var reasoning=ReadString(root,"delta");
            if(reasoning.Length==0&&root.TryGetProperty("delta",out var value)&&value.ValueKind==JsonValueKind.Object)
                reasoning=ReadString(value,"text");
            delta=new(string.Empty,reasoning);return true;
        }

        // Anthropic-style event names are also emitted by a number of
        // OpenAI-compatible proxies.  Their payload keeps the text under a
        // nested delta object.
        if(type.Equals("content_block_delta",StringComparison.OrdinalIgnoreCase)||type.Equals("message_delta",StringComparison.OrdinalIgnoreCase))
        {
            if(root.TryGetProperty("delta",out var value)&&value.ValueKind==JsonValueKind.Object)
            {
                var deltaType=ReadString(value,"type");
                if(deltaType is "text_delta" or "output_text_delta")
                    delta=new(ReadString(value,"text"),string.Empty);
                else if(deltaType.Contains("thinking",StringComparison.OrdinalIgnoreCase)||deltaType.Contains("reasoning",StringComparison.OrdinalIgnoreCase))
                {
                    var thinking=ReadString(value,"thinking");
                    if(thinking.Length==0)thinking=ReadString(value,"text");
                    delta=new(string.Empty,thinking);
                }
                var stopReason=ReadString(value,"stop_reason");
                if(stopReason.Length>0)
                {
                    done=true;
                    truncated=stopReason.Equals("max_tokens",StringComparison.OrdinalIgnoreCase)||stopReason.Equals("length",StringComparison.OrdinalIgnoreCase);
                }
            }
            return true;
        }

        // Responses emits response.completed/response.done after the final
        // text event.  Do not treat output_text.done as terminal because a
        // multi-output response can still contain another item afterwards.
        if(type.Equals("response.completed",StringComparison.OrdinalIgnoreCase)||
           type.Equals("response.done",StringComparison.OrdinalIgnoreCase)||
           type.Equals("message_stop",StringComparison.OrdinalIgnoreCase)||
           type.Equals("response.incomplete",StringComparison.OrdinalIgnoreCase))
        {
            done=true;truncated=type.Equals("response.incomplete",StringComparison.OrdinalIgnoreCase);return true;
        }

        // Unknown but well-formed events are valid SSE records; consume them
        // so diagnostics can distinguish an unsupported event stream from a
        // malformed/truncated response.
        return true;
    }

    private static string ReadString(JsonElement value,string name)=>value.TryGetProperty(name,out var property)&&property.ValueKind==JsonValueKind.String?property.GetString()??string.Empty:string.Empty;

    internal static (string Content,string Reasoning) ReadContentParts(JsonElement value)
    {
        if(value.ValueKind!=JsonValueKind.Object)return (string.Empty,string.Empty);
        if(!value.TryGetProperty("content",out var content))
        {
            // A few OpenAI-compatible gateways put their final text in these
            // aliases. They are accepted only as explicit text fields; no
            // reasoning field is ever promoted into an answer.
            var outputText=ReadString(value,"output_text");
            if(outputText.Length==0)outputText=ReadString(value,"text");
            return (outputText,string.Empty);
        }
        if(content.ValueKind==JsonValueKind.String)return (content.GetString()??string.Empty,string.Empty);
        if(content.ValueKind==JsonValueKind.Object)
        {
            // A gateway may wrap an otherwise plain answer in a single object.
            // Only an absent discriminator permits this compatibility path;
            // unknown or malformed declared types must never become answer text.
            if(!content.TryGetProperty("type",out _))
            {
                var objectText=ReadString(content,"text");
                if(objectText.Length==0)objectText=ReadString(content,"content");
                return (objectText,string.Empty);
            }
            var objectTextParts=new StringBuilder();
            var objectReasoningParts=new StringBuilder();
            AppendTypedContent(content,objectTextParts,objectReasoningParts);
            return (objectTextParts.ToString(),objectReasoningParts.ToString());
        }
        if(content.ValueKind!=JsonValueKind.Array)return (string.Empty,string.Empty);

        var text=new StringBuilder();
        var reasoning=new StringBuilder();
        foreach(var chunk in content.EnumerateArray())
            AppendTypedContent(chunk,text,reasoning);
        return (text.ToString(),reasoning.ToString());
    }

    private static void AppendTypedContent(JsonElement chunk,StringBuilder text,StringBuilder reasoning)
    {
        if(chunk.ValueKind!=JsonValueKind.Object)return;
        switch(ReadString(chunk,"type"))
        {
            case "text":
            case "output_text":
            case "text_delta":
                // Text content blocks, including Claude's text_delta, use text.
                // A Responses response.output_text.delta event is a different
                // envelope and must not be inferred from a generic delta field.
                text.Append(ReadString(chunk,"text"));
                break;
            case "thinking":
                // Mistral streams ThinkChunk.thinking as TextChunk[], including
                // a mixed thinking/text event when the final answer begins.
                if(!chunk.TryGetProperty("thinking",out var thoughts)||thoughts.ValueKind!=JsonValueKind.Array)break;
                foreach(var thought in thoughts.EnumerateArray())
                    if(thought.ValueKind==JsonValueKind.Object&&ReadString(thought,"type")=="text")
                        reasoning.Append(ReadString(thought,"text"));
                break;
        }
    }

    internal static string ReadReasoningDetails(JsonElement value)
    {
        if(!value.TryGetProperty("reasoning_details",out var details))return string.Empty;
        if(details.ValueKind==JsonValueKind.String)return details.GetString()??string.Empty;
        if(details.ValueKind!=JsonValueKind.Array)return string.Empty;
        return string.Concat(details.EnumerateArray().Select(item=>item.ValueKind==JsonValueKind.Object?ReadString(item,"text"):string.Empty));
    }
}
