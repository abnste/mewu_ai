using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal readonly record struct AnnotationUpdateResult(IReadOnlyList<AiAnnotation> Annotations,bool Changed,bool Replaced);

internal static class AnnotationUpdateService
{
    internal static AnnotationUpdateResult Apply(IReadOnlyList<AiAnnotation> existing,IReadOnlyList<AiAnnotation> incoming,AiAnnotationUpdateMode mode,bool isVideo)
    {
        ArgumentNullException.ThrowIfNull(existing);ArgumentNullException.ThrowIfNull(incoming);
        // A response without executable annotations is never allowed to erase
        // the current canvas. Explicit replacement becomes effective only when
        // the model actually returned a valid replacement set.
        if(mode==AiAnnotationUpdateMode.Preserve||incoming.Count==0)return new(existing.ToArray(),false,false);
        var replacing=mode==AiAnnotationUpdateMode.Replace;var candidates=replacing?incoming:existing.Concat(incoming).ToArray();
        var merged=AnnotationPostProcessor.Process(candidates,isVideo,out _).ToArray();
        if(!replacing&&existing.SequenceEqual(merged))return new(existing.ToArray(),false,false);
        return new(merged,true,replacing);
    }
}
