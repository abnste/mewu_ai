// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

/// <summary>Reconstructs a smooth screenshot background from pixels outside the erased rectangle.</summary>
internal static class SeamlessEraseService
{
    private const string MissingBackgroundMessage = "Seamless erase requires surrounding background pixels.";
    private enum Side { Top, Bottom, Left, Right }
    private sealed record BoundaryStrip(int Distance, byte[] Pixels);
    private sealed record BoundaryProfile(double Coordinate, byte[] Pixels);

    private readonly record struct Sample(double X, double Y, byte B, byte G, byte R, byte A)
    {
        internal double Channel(int channel) => channel switch { 0 => B, 1 => G, 2 => R, _ => A };
    }

    internal static BitmapSource CreatePatch(BitmapSource source, Int32Rect region) => CreatePatch(source, region, int.MaxValue);

    // The preview samples the same original boundary, but bounds bitmap allocation
    // during a drag. Committing always generates the full physical-pixel patch.
    internal static BitmapSource CreatePreviewPatch(BitmapSource source, Int32Rect region) => CreatePatch(source, region, 256);

    /// <summary>Separates local screenshot content from its reconstructed background.</summary>
    internal static BitmapSource ExtractContent(BitmapSource source, Int32Rect region, BitmapSource background)
    {
        ArgumentNullException.ThrowIfNull(source);
        ArgumentNullException.ThrowIfNull(background);
        if (region.X < 0 || region.Y < 0 || region.Width <= 0 || region.Height <= 0 ||
            (long)region.X + region.Width > source.PixelWidth || (long)region.Y + region.Height > source.PixelHeight)
            throw new ArgumentOutOfRangeException(nameof(region));
        if (background.PixelWidth != region.Width || background.PixelHeight != region.Height)
            throw new ArgumentException("The reconstructed background must match the full-resolution extraction region.", nameof(background));

        // Work in straight BGRA, not premultiplied PBGRA. The output owns its
        // pixels and does not retain a CroppedBitmap/decoder for the whole source.
        var original = source.Format == PixelFormats.Bgra32 ? source : new FormatConvertedBitmap(source, PixelFormats.Bgra32, null, 0);
        var backdrop = background.Format == PixelFormats.Bgra32 ? background : new FormatConvertedBitmap(background, PixelFormats.Bgra32, null, 0);
        var width = region.Width;
        var height = region.Height;
        var stride = checked(width * 4);
        var length = checked(stride * height);
        var observed = new byte[length];
        var clean = new byte[length];
        var output = new byte[length];
        var contrast = new byte[checked(width * height)];
        const int noiseThreshold = 3;
        const int radius = 3;
        try
        {
            original.CopyPixels(region, observed, stride, 0);
            backdrop.CopyPixels(clean, stride, 0);
            for (var pixel = 0; pixel < contrast.Length; pixel++)
            {
                var at = pixel * 4;
                if (observed[at + 3] == 0) continue;
                contrast[pixel] = (byte)Math.Max(Math.Abs(observed[at] - clean[at]),
                    Math.Max(Math.Abs(observed[at + 1] - clean[at + 1]), Math.Abs(observed[at + 2] - clean[at + 2])));
            }
            for (var y = 0; y < height; y++)
            {
                for (var x = 0; x < width; x++)
                {
                    var pixel = y * width + x;
                    var at = pixel * 4;
                    var sourceAlpha = observed[at + 3];
                    var backgroundAlpha = clean[at + 3];
                    if (sourceAlpha == 0) continue;
                    if (contrast[pixel] <= noiseThreshold && Math.Abs(sourceAlpha - backgroundAlpha) <= noiseThreshold) continue;
                    // With a nonopaque backdrop, an increase in alpha provides
                    // the missing coverage directly from the source-over equation.
                    // A transparent backdrop also preserves black foregrounds,
                    // which need not have any RGB difference at all.
                    if (backgroundAlpha < 255 && sourceAlpha > backgroundAlpha)
                    {
                        var coverage = (sourceAlpha - backgroundAlpha) / (255d - backgroundAlpha);
                        var alpha = (byte)Math.Clamp(Math.Round(coverage * 255), 1, 255);
                        coverage = alpha / 255d;
                        for (var channel = 0; channel < 3; channel++)
                            output[at + channel] = ToByte((observed[at + channel] * sourceAlpha / 255d -
                                (1 - coverage) * clean[at + channel] * backgroundAlpha / 255d) / coverage);
                        output[at + 3] = alpha;
                        continue;
                    }
                    var strength = contrast[pixel];
                    if (strength <= noiseThreshold) continue;
                    var db = observed[at] - clean[at];
                    var dg = observed[at + 1] - clean[at + 1];
                    var dr = observed[at + 2] - clean[at + 2];
                    var alphaEstimate = 1d;
                    // A nearby, stronger observation in the same color direction
                    // estimates a stroke's solid core. A fixed 7x7 neighbourhood
                    // bounds the work; unrelated-colored neighbours are rejected.
                    if (strength < 255 && NearContentBoundary(contrast, width, height, x, y, noiseThreshold))
                    {
                        for (var nearbyY = Math.Max(0, y - radius); nearbyY <= Math.Min(height - 1, y + radius); nearbyY++)
                        {
                            for (var nearbyX = Math.Max(0, x - radius); nearbyX <= Math.Min(width - 1, x + radius); nearbyX++)
                            {
                                var nearbyPixel = nearbyY * width + nearbyX;
                                if (contrast[nearbyPixel] <= strength) continue;
                                var nearby = nearbyPixel * 4;
                                if (observed[nearby + 3] < sourceAlpha) continue;
                                var fb = observed[nearby] - clean[at];
                                var fg = observed[nearby + 1] - clean[at + 1];
                                var fr = observed[nearby + 2] - clean[at + 2];
                                var norm = fb * fb + fg * fg + fr * fr;
                                if (norm == 0) continue;
                                var estimate = (db * fb + dg * fg + dr * fr) / (double)norm;
                                if (estimate <= 0 || estimate >= alphaEstimate) continue;
                                var residual = Math.Max(Math.Abs(db - estimate * fb),
                                    Math.Max(Math.Abs(dg - estimate * fg), Math.Abs(dr - estimate * fr)));
                                if (residual <= Math.Max(2, strength * .025) &&
                                    HasForegroundPath(contrast, width, x, y, nearbyX, nearbyY, noiseThreshold)) alphaEstimate = estimate;
                            }
                        }
                    }
                    // C = alpha F + (1-alpha) B. Infer F, rather than retaining
                    // C under partial alpha (which leaves a halo of the old B).
                    // Gamut bounds prevent borrowing a darker/lighter neighbour
                    // from producing invalid foreground channels after unmixing.
                    var minimumAlpha = 0d;
                    for (var channel = 0; channel < 3; channel++)
                    {
                        var difference = observed[at + channel] - clean[at + channel];
                        if (difference > 0) minimumAlpha = Math.Max(minimumAlpha, difference / (255d - clean[at + channel]));
                        else if (difference < 0) minimumAlpha = Math.Max(minimumAlpha, -difference / (double)clean[at + channel]);
                    }
                    var coverageByte = (byte)Math.Clamp(Math.Max(Math.Ceiling(minimumAlpha * 255), Math.Round(alphaEstimate * 255)), 1, 255);
                    var coverageEstimate = coverageByte / 255d;
                    for (var channel = 0; channel < 3; channel++)
                        output[at + channel] = ToByte(clean[at + channel] + (observed[at + channel] - clean[at + channel]) / coverageEstimate);
                    output[at + 3] = (byte)Math.Clamp(Math.Round(coverageEstimate * sourceAlpha), 1, 255);
                }
            }
            var result = BitmapSource.Create(width, height, source.DpiX, source.DpiY, PixelFormats.Bgra32, null, output, stride);
            result.Freeze();
            return result;
        }
        finally
        {
            Array.Clear(observed);
            Array.Clear(clean);
            Array.Clear(output);
            Array.Clear(contrast);
        }
    }

    private static byte ToByte(double value) => (byte)Math.Clamp(Math.Round(value), 0, 255);

    private static bool NearContentBoundary(byte[] contrast, int width, int height, int x, int y, int threshold)
    {
        // Keep solid interiors opaque, including intentionally shaded patterns.
        // Only the narrow band next to observed background needs edge matting.
        for (var row = Math.Max(0, y - 2); row <= Math.Min(height - 1, y + 2); row++)
            for (var column = Math.Max(0, x - 2); column <= Math.Min(width - 1, x + 2); column++)
                if (contrast[row * width + column] <= threshold) return true;
        return false;
    }

    private static bool HasForegroundPath(byte[] contrast, int width, int x, int y, int targetX, int targetY, int threshold)
    {
        // Do not use a separate darker letter across a clear background gap as
        // the solid core of a nearby light letter. This walk is at most 3 pixels.
        var steps = Math.Max(Math.Abs(targetX - x), Math.Abs(targetY - y));
        for (var step = 1; step < steps; step++)
        {
            var column = x + (int)Math.Round((targetX - x) * step / (double)steps);
            var row = y + (int)Math.Round((targetY - y) * step / (double)steps);
            if (contrast[row * width + column] <= threshold) return false;
        }
        return true;
    }

    private static BitmapSource CreatePatch(BitmapSource source, Int32Rect region, int maximumDimension)
    {
        ArgumentNullException.ThrowIfNull(source);
        if (region.X < 0 || region.Y < 0 || region.Width <= 0 || region.Height <= 0 ||
            (long)region.X + region.Width > source.PixelWidth || (long)region.Y + region.Height > source.PixelHeight)
            throw new ArgumentOutOfRangeException(nameof(region));
        var formatted = source.Format == PixelFormats.Bgra32 ? source : new FormatConvertedBitmap(source, PixelFormats.Bgra32, null, 0);
        var samples = new Sample[4 * 3 * 128];
        var strips = new List<BoundaryStrip>[] { [], [], [], [] };
        var profiles = new BoundaryProfile?[4];
        double[,]? coefficients = null;
        var count = 0;
        try
        {
            // Bounded perimeter strips, never the text being removed. Extra strips
            // and robust fitting keep an occasional neighbouring glyph from tinting
            // the result. Coordinates are normalized for stable least squares.
            foreach (var distance in new[] { 1, 3, 5 })
            {
                if (region.Y >= distance) ReadStrip(Side.Top, distance, region.X, region.Y - distance, region.Width, 1);
                if (region.Y + region.Height - 1 + distance < source.PixelHeight)
                    ReadStrip(Side.Bottom, distance, region.X, region.Y + region.Height - 1 + distance, region.Width, 1);
                if (region.X >= distance) ReadStrip(Side.Left, distance, region.X - distance, region.Y, 1, region.Height);
                if (region.X + region.Width - 1 + distance < source.PixelWidth)
                    ReadStrip(Side.Right, distance, region.X + region.Width - 1 + distance, region.Y, 1, region.Height);
            }
            // A fully masked source contains no background evidence. In particular,
            // its inside edge must not become a donor carrying the removed text.
            if (count == 0) throw new InvalidOperationException(MissingBackgroundMessage);
            coefficients = FitBackground(samples, count);
            var usePlane = SupportsPlane(samples, count, coefficients);
            if (!usePlane)
            {
                // Full-resolution edge profiles retain hard panel boundaries.
                // Their storage scales with the perimeter, not the image area.
                for (var side = 0; side < profiles.Length; side++)
                    profiles[side] = CreateProfile(strips[side], (Side)side, region);
            }
            var verticalError = PairError(profiles[(int)Side.Top], profiles[(int)Side.Bottom]);
            var horizontalError = PairError(profiles[(int)Side.Left], profiles[(int)Side.Right]);
            var useVertical = double.IsFinite(verticalError) &&
                (!double.IsFinite(horizontalError) || verticalError <= horizontalError * .75);
            var useHorizontal = !useVertical && double.IsFinite(horizontalError) &&
                (!double.IsFinite(verticalError) || horizontalError <= verticalError * .75);
            var ratio = Math.Min(1d, maximumDimension / (double)Math.Max(region.Width, region.Height));
            var width = Math.Max(1, (int)Math.Round(region.Width * ratio));
            var height = Math.Max(1, (int)Math.Round(region.Height * ratio));
            var stride = checked(width * 4);
            var pixels = new byte[checked(stride * height)];
            try
            {
                for (var y = 0; y < height; y++)
                {
                    var sourceY = (y + .5) * region.Height / height - .5;
                    var normalizedY = Normalize(sourceY, region.Height);
                    for (var x = 0; x < width; x++)
                    {
                        var sourceX = (x + .5) * region.Width / width - .5;
                        var normalizedX = Normalize(sourceX, region.Width);
                        var offset = y * stride + x * 4;
                        for (var channel = 0; channel < 4; channel++)
                        {
                            var value = usePlane
                                ? coefficients[channel, 0] + coefficients[channel, 1] * normalizedX + coefficients[channel, 2] * normalizedY
                                : EvaluateBoundary(profiles, sourceX, sourceY, channel, useVertical, useHorizontal);
                            pixels[offset + channel] = (byte)Math.Clamp(Math.Round(value), 0, 255);
                        }
                    }
                }
                var bitmap = BitmapSource.Create(width, height, source.DpiX, source.DpiY, PixelFormats.Bgra32, null, pixels, stride);
                bitmap.Freeze();
                return bitmap;
            }
            finally { Array.Clear(pixels); }
        }
        finally
        {
            Array.Clear(samples);
            if (coefficients is not null) Array.Clear(coefficients);
            foreach (var side in strips)
                foreach (var strip in side) Array.Clear(strip.Pixels);
            foreach (var profile in profiles)
                if (profile is not null) Array.Clear(profile.Pixels);
        }

        void ReadStrip(Side side, int distance, int left, int top, int width, int height)
        {
            var stride = checked(width * 4);
            var pixels = new byte[checked(stride * height)];
            strips[(int)side].Add(new BoundaryStrip(distance, pixels));
            formatted.CopyPixels(new Int32Rect(left, top, width, height), pixels, stride, 0);
            var length = Math.Max(width, height);
            var entries = Math.Min(128, length);
            for (var index = 0; index < entries; index++)
            {
                var position = entries == 1 ? 0 : (int)Math.Round(index * (length - 1d) / (entries - 1));
                var x = width == 1 ? 0 : position;
                var y = height == 1 ? 0 : position;
                var offset = y * stride + x * 4;
                samples[count++] = new Sample(Normalize(left + x - region.X, region.Width), Normalize(top + y - region.Y, region.Height),
                    pixels[offset], pixels[offset + 1], pixels[offset + 2], pixels[offset + 3]);
            }
        }
    }

    private static double Normalize(double position, int length) => (position - (length - 1d) / 2) / Math.Max(1, length / 2d);

    private static bool SupportsPlane(Sample[] samples, int count, double[,] coefficients)
    {
        var supported = 0;
        var squaredError = 0d;
        for (var index = 0; index < count; index++)
        {
            var residual = Residual(samples[index], coefficients);
            if (!double.IsFinite(residual)) return false;
            if (residual > 6) continue;
            supported++;
            squaredError += residual * residual;
        }
        // A robust fit must still explain almost all observations. Otherwise an
        // unrelated panel can be classified as an outlier and painted over.
        return supported >= count * .9 && squaredError <= supported * 9d;
    }

    private static double Residual(Sample sample, double[,] coefficients)
    {
        var residual = 0d;
        for (var channel = 0; channel < 4; channel++)
            residual = Math.Max(residual, Math.Abs(sample.Channel(channel) - coefficients[channel, 0] -
                coefficients[channel, 1] * sample.X - coefficients[channel, 2] * sample.Y));
        return residual;
    }

    private static BoundaryProfile? CreateProfile(List<BoundaryStrip> strips, Side side, Int32Rect region)
    {
        if (strips.Count == 0) return null;
        var pixels = new byte[strips[0].Pixels.Length];
        for (var index = 0; index < pixels.Length; index++)
        {
            var first = strips[0].Pixels[index];
            if (strips.Count == 1) pixels[index] = first;
            else if (strips.Count == 2)
                pixels[index] = (byte)((first + strips[1].Pixels[index] + 1) / 2);
            else
            {
                var second = strips[1].Pixels[index];
                var third = strips[2].Pixels[index];
                pixels[index] = (byte)(first + second + third - Math.Min(first, Math.Min(second, third)) - Math.Max(first, Math.Max(second, third)));
            }
        }
        var distance = strips.Average(strip => strip.Distance);
        var coordinate = side switch
        {
            Side.Top or Side.Left => -distance,
            Side.Bottom => region.Height - 1 + distance,
            _ => region.Width - 1 + distance
        };
        return new BoundaryProfile(coordinate, pixels);
    }

    private static double PairError(BoundaryProfile? first, BoundaryProfile? second)
    {
        if (first is null || second is null) return double.PositiveInfinity;
        var error = 0d;
        for (var offset = 0; offset < first.Pixels.Length; offset += 4)
        {
            var difference = 0;
            for (var channel = 0; channel < 4; channel++)
                difference = Math.Max(difference, Math.Abs(first.Pixels[offset + channel] - second.Pixels[offset + channel]));
            error += difference;
        }
        return error / (first.Pixels.Length / 4);
    }

    private static double EvaluateBoundary(BoundaryProfile?[] profiles, double x, double y, int channel,
        bool useVertical, bool useHorizontal)
    {
        var top = profiles[(int)Side.Top];
        var bottom = profiles[(int)Side.Bottom];
        var left = profiles[(int)Side.Left];
        var right = profiles[(int)Side.Right];
        if (useVertical) return InterpolatePair(top!, bottom!, x, y, channel);
        if (useHorizontal) return InterpolatePair(left!, right!, y, x, channel);

        // Ambiguous or one-sided surroundings use bounded convex interpolation.
        // This never invents colors beyond observed boundary values, but does not
        // claim to reconstruct arbitrary photographic texture or hidden objects.
        var total = 0d;
        var weightSum = 0d;
        for (var side = 0; side < profiles.Length; side++)
        {
            var profile = profiles[side];
            if (profile is null) continue;
            var horizontalEdge = side < 2;
            var distance = Math.Abs((horizontalEdge ? y : x) - profile.Coordinate);
            var weight = 1d / Math.Max(1, distance);
            total += ReadProfile(profile, horizontalEdge ? x : y, channel) * weight;
            weightSum += weight;
        }
        return total / weightSum;
    }

    private static double InterpolatePair(BoundaryProfile first, BoundaryProfile second, double along,
        double across, int channel)
    {
        var fraction = Math.Clamp((across - first.Coordinate) / (second.Coordinate - first.Coordinate), 0, 1);
        return ReadProfile(first, along, channel) * (1 - fraction) + ReadProfile(second, along, channel) * fraction;
    }

    private static double ReadProfile(BoundaryProfile profile, double position, int channel)
    {
        var last = profile.Pixels.Length / 4 - 1;
        position = Math.Clamp(position, 0, last);
        var before = (int)position;
        var after = Math.Min(before + 1, last);
        var fraction = position - before;
        return profile.Pixels[before * 4 + channel] * (1 - fraction) + profile.Pixels[after * 4 + channel] * fraction;
    }

    private static double[,] FitBackground(Sample[] samples, int count)
    {
        var included = new bool[count];
        Array.Fill(included, true);
        var residuals = new double[count];
        var sorted = new double[count];
        var coefficients = new double[4, 3];
        for (var pass = 0; pass < 4; pass++)
        {
            var meanX = 0d;
            var meanY = 0d;
            var includedCount = 0;
            for (var index = 0; index < count; index++)
            {
                if (!included[index]) continue;
                meanX += samples[index].X;
                meanY += samples[index].Y;
                includedCount++;
            }
            meanX /= Math.Max(1, includedCount);
            meanY /= Math.Max(1, includedCount);
            // Small regularization makes one-sided / one-pixel boundaries safe;
            // centering first prefers a constant on an unobservable axis rather
            // than trading off its slope against the color intercept.
            var matrix = new double[3, 7];
            matrix[0, 0] = matrix[1, 1] = matrix[2, 2] = 1e-8;
            for (var index = 0; index < count; index++)
            {
                if (!included[index]) continue;
                var sample = samples[index];
                var x = sample.X - meanX;
                var y = sample.Y - meanY;
                for (var row = 0; row < 3; row++)
                {
                    var a = row == 0 ? 1 : row == 1 ? x : y;
                    matrix[row, 0] += a;
                    matrix[row, 1] += a * x;
                    matrix[row, 2] += a * y;
                    for (var channel = 0; channel < 4; channel++) matrix[row, 3 + channel] += a * sample.Channel(channel);
                }
            }
            // Pivoted elimination solves all four channel planes together.
            for (var column = 0; column < 3; column++)
            {
                var pivot = column;
                for (var row = column + 1; row < 3; row++)
                    if (Math.Abs(matrix[row, column]) > Math.Abs(matrix[pivot, column])) pivot = row;
                for (var c = 0; c < 7; c++) (matrix[column, c], matrix[pivot, c]) = (matrix[pivot, c], matrix[column, c]);
                var divisor = matrix[column, column];
                for (var c = column; c < 7; c++) matrix[column, c] /= divisor;
                for (var row = 0; row < 3; row++)
                {
                    if (row == column) continue;
                    var scale = matrix[row, column];
                    for (var c = column; c < 7; c++) matrix[row, c] -= scale * matrix[column, c];
                }
            }
            for (var channel = 0; channel < 4; channel++)
            {
                coefficients[channel, 1] = matrix[1, channel + 3];
                coefficients[channel, 2] = matrix[2, channel + 3];
                coefficients[channel, 0] = matrix[0, channel + 3] - coefficients[channel, 1] * meanX - coefficients[channel, 2] * meanY;
            }
            Array.Clear(matrix);
            if (pass == 3) break;
            for (var index = 0; index < count; index++)
            {
                var residual = Residual(samples[index], coefficients);
                residuals[index] = sorted[index] = residual;
            }
            Array.Sort(sorted);
            var threshold = Math.Max(3, sorted[count / 2] * 2.5);
            for (var index = 0; index < count; index++) included[index] = residuals[index] <= threshold;
        }
        Array.Clear(residuals); Array.Clear(sorted);
        return coefficients;
    }
}
