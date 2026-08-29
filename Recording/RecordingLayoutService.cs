using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Recording;

public static class RecordingLayoutService
{
    public static IReadOnlyList<RecordingSlice> CreateSlices(ScreenRect region,IEnumerable<ScreenRect> displays)
    {
        var slices=new List<RecordingSlice>();
        foreach(var display in displays)
        {
            var source=region.Intersect(display);
            if(source.IsEmpty)continue;
            slices.Add(new RecordingSlice(source,display,new ScreenRect(source.X-region.X,source.Y-region.Y,source.Width,source.Height)));
        }
        return slices;
    }
}

public readonly record struct RecordingSlice(ScreenRect Source,ScreenRect Display,ScreenRect Output);
