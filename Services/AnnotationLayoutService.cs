namespace mewu_ai_Assistant.Services;

public static class AnnotationLayoutService
{
    public static double FindCardTop(double preferred,double minimum,double maximum,double minimumDistance,IReadOnlyCollection<double> occupied)
    {
        maximum=Math.Max(minimum,maximum);
        preferred=Math.Clamp(preferred,minimum,maximum);
        minimumDistance=Math.Max(0,minimumDistance);
        if(occupied.Count==0||occupied.All(value=>Math.Abs(value-preferred)>=minimumDistance))return preferred;

        var candidates=new List<double>{preferred,minimum,maximum};
        foreach(var value in occupied)
        {
            candidates.Add(Math.Clamp(value-minimumDistance,minimum,maximum));
            candidates.Add(Math.Clamp(value+minimumDistance,minimum,maximum));
        }

        var ordered=candidates.Distinct().OrderBy(value=>Math.Abs(value-preferred)).ToList();
        var available=ordered.FirstOrDefault(candidate=>occupied.All(value=>Math.Abs(value-candidate)>=minimumDistance),double.NaN);
        if(!double.IsNaN(available))return available;
        return ordered.OrderByDescending(candidate=>occupied.Min(value=>Math.Abs(value-candidate))).ThenBy(candidate=>Math.Abs(candidate-preferred)).First();
    }
}
