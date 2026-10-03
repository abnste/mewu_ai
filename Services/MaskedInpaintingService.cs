// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections.Generic;
using System.Threading;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

/// <summary>Local, mask-limited screenshot repair; no model, network or source mutation.</summary>
internal static class MaskedInpaintingService
{
    private const int MaximumWorkingPixels = 16 * 1024 * 1024;
    private const int ContextMargin = 12;
    private const byte Original = 0, Unknown = 1, Frontier = 2, Repaired = 3;
    private const string MissingBackgroundMessage = "Masked repair requires surrounding background pixels.";
    private readonly record struct BoundarySample(double X, double Y, byte B, byte G, byte R, byte A)
    {
        internal double Channel(int channel) => channel switch { 0 => B, 1 => G, 2 => R, _ => A };
    }
    private readonly record struct Neighbour(int X, int Y, double Weight);
    private static readonly Neighbour[] Neighbours = CreateNeighbours();

    // The mask is tightly packed ROI coverage, not a full-image mask. Every
    // nonzero entry is unknown, even a soft brush edge; only zero is a donor.
    internal static BitmapSource CreatePatch(BitmapSource source, Int32Rect region, byte[] mask,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(source);
        ArgumentNullException.ThrowIfNull(mask);
        if (region.X < 0 || region.Y < 0 || region.Width <= 0 || region.Height <= 0 ||
            (long)region.X + region.Width > source.PixelWidth || (long)region.Y + region.Height > source.PixelHeight)
            throw new ArgumentOutOfRangeException(nameof(region));
        if (mask.LongLength != (long)region.Width * region.Height)
            throw new ArgumentException("The mask must contain one coverage byte per region pixel.", nameof(mask));
        if (mask.LongLength > MaximumWorkingPixels)
            throw new InvalidOperationException("The masked repair region is too large.");
        cancellationToken.ThrowIfCancellationRequested();
        var coverage = (byte[])mask.Clone();
        var output = new byte[checked(coverage.Length * 4)];
        byte[]? working = null;
        byte[]? state = null;
        double[,]? plane = null;
        try
        {
            var minX = region.Width; var minY = region.Height; var maxX = -1; var maxY = -1;
            var missing = 0;
            for (var y = 0; y < region.Height; y++)
            {
                cancellationToken.ThrowIfCancellationRequested();
                for (var x = 0; x < region.Width; x++)
                {
                    if (coverage[y * region.Width + x] == 0) continue;
                    minX = Math.Min(minX, x); minY = Math.Min(minY, y);
                    maxX = Math.Max(maxX, x); maxY = Math.Max(maxY, y); missing++;
                }
            }
            if (missing == 0) return Finish();
            var left = Math.Max(0, region.X + minX - ContextMargin);
            var top = Math.Max(0, region.Y + minY - ContextMargin);
            var right = (int)Math.Min(source.PixelWidth, (long)region.X + maxX + 1 + ContextMargin);
            var bottom = (int)Math.Min(source.PixelHeight, (long)region.Y + maxY + 1 + ContextMargin);
            var width = right - left; var height = bottom - top;
            if ((long)width * height > MaximumWorkingPixels)
                throw new InvalidOperationException("The masked repair region is too large.");
            state = new byte[checked(width * height)];
            working = new byte[checked(state.Length * 4)];
            var formatted = source.Format == PixelFormats.Bgra32 ? source : new FormatConvertedBitmap(source, PixelFormats.Bgra32, null, 0);
            formatted.CopyPixels(new Int32Rect(left, top, width, height), working, checked(width * 4), 0);
            for (var y = minY; y <= maxY; y++)
            {
                cancellationToken.ThrowIfCancellationRequested();
                for (var x = minX; x <= maxX; x++)
                {
                    if (coverage[y * region.Width + x] == 0) continue;
                    var index = (region.Y + y - top) * width + region.X + x - left;
                    state[index] = Unknown;
                    // Erased source colors can never affect fitting, gradients,
                    // propagation or the final patch, including partial coverage.
                    var at = index * 4;
                    working[at] = working[at + 1] = working[at + 2] = working[at + 3] = 0;
                }
            }
            plane = FitBoundary(working, state, width, height, cancellationToken);
            if (plane is null) RepairFromBoundary(working, state, width, height, missing, cancellationToken);
            for (var y = minY; y <= maxY; y++)
            {
                cancellationToken.ThrowIfCancellationRequested();
                for (var x = minX; x <= maxX; x++)
                {
                    var outputPixel = y * region.Width + x;
                    if (coverage[outputPixel] == 0) continue;
                    var localX = region.X + x - left; var localY = region.Y + y - top;
                    var inputAt = (localY * width + localX) * 4;
                    var outputAt = outputPixel * 4;
                    for (var channel = 0; channel < 4; channel++)
                    {
                        var value = plane is null ? working[inputAt + channel] : ToByte(plane[channel, 0] +
                            plane[channel, 1] * Normalize(localX, width) + plane[channel, 2] * Normalize(localY, height));
                        output[outputAt + channel] = channel == 3 ? (byte)((value * coverage[outputPixel] + 127) / 255) : value;
                    }
                    if (output[outputAt + 3] == 0) Array.Clear(output, outputAt, 4);
                }
            }
            return Finish();
        }
        finally
        {
            Array.Clear(coverage); Array.Clear(output);
            if (working is not null) Array.Clear(working);
            if (state is not null) Array.Clear(state);
            if (plane is not null) Array.Clear(plane);
        }

        BitmapSource Finish()
        {
            cancellationToken.ThrowIfCancellationRequested();
            var bitmap = BitmapSource.Create(region.Width, region.Height, source.DpiX, source.DpiY,
                PixelFormats.Bgra32, null, output, checked(region.Width * 4));
            bitmap.Freeze();
            cancellationToken.ThrowIfCancellationRequested();
            return bitmap;
        }
    }

    private static double[,]? FitBoundary(byte[] pixels, byte[] state, int width, int height, CancellationToken token)
    {
        var boundaryCount = 0;
        for (var y = 0; y < height; y++)
        {
            token.ThrowIfCancellationRequested();
            for (var x = 0; x < width; x++)
                if (IsBoundary(state, width, height, x, y)) boundaryCount++;
        }
        if (boundaryCount == 0) throw new InvalidOperationException(MissingBackgroundMessage);
        // Uniform deterministic sampling covers even a long, sparse brush path.
        var step = Math.Max(1, (boundaryCount + 4095) / 4096);
        var samples = new BoundarySample[(boundaryCount + step - 1) / step];
        var seen = 0; var count = 0;
        var included = new bool[samples.Length]; Array.Fill(included, true);
        var residuals = new double[samples.Length]; var sorted = new double[samples.Length];
        var coefficients = new double[4, 3];
        var accepted = false;
        try
        {
            for (var y = 0; y < height; y++)
            {
                token.ThrowIfCancellationRequested();
                for (var x = 0; x < width; x++)
                {
                    if (!IsBoundary(state, width, height, x, y) || seen++ % step != 0) continue;
                    var at = (y * width + x) * 4;
                    samples[count++] = new BoundarySample(Normalize(x, width), Normalize(y, height),
                        pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]);
                }
            }
            for (var pass = 0; pass < 4; pass++)
            {
                token.ThrowIfCancellationRequested();
                var meanX = 0d; var meanY = 0d; var used = 0;
                for (var i = 0; i < count; i++)
                    if (included[i]) { meanX += samples[i].X; meanY += samples[i].Y; used++; }
                meanX /= Math.Max(1, used); meanY /= Math.Max(1, used);
                var matrix = new double[3, 7];
                matrix[0, 0] = matrix[1, 1] = matrix[2, 2] = 1e-10;
                for (var i = 0; i < count; i++)
                {
                    if (!included[i]) continue;
                    var sample = samples[i]; var x = sample.X - meanX; var y = sample.Y - meanY;
                    for (var row = 0; row < 3; row++)
                    {
                        var factor = row == 0 ? 1 : row == 1 ? x : y;
                        matrix[row, 0] += factor; matrix[row, 1] += factor * x; matrix[row, 2] += factor * y;
                        for (var channel = 0; channel < 4; channel++) matrix[row, 3 + channel] += factor * sample.Channel(channel);
                    }
                }
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
                        var factor = matrix[row, column];
                        for (var c = column; c < 7; c++) matrix[row, c] -= factor * matrix[column, c];
                    }
                }
                for (var channel = 0; channel < 4; channel++)
                {
                    coefficients[channel, 1] = matrix[1, channel + 3]; coefficients[channel, 2] = matrix[2, channel + 3];
                    coefficients[channel, 0] = matrix[0, channel + 3] - coefficients[channel, 1] * meanX - coefficients[channel, 2] * meanY;
                }
                Array.Clear(matrix);
                for (var i = 0; i < count; i++)
                {
                    var error = 0d;
                    for (var channel = 0; channel < 4; channel++)
                        error = Math.Max(error, Math.Abs(samples[i].Channel(channel) - coefficients[channel, 0] -
                            coefficients[channel, 1] * samples[i].X - coefficients[channel, 2] * samples[i].Y));
                    residuals[i] = sorted[i] = error;
                }
                if (pass == 3) break;
                Array.Sort(sorted); var threshold = Math.Max(3, sorted[count / 2] * 2.5);
                for (var i = 0; i < count; i++) included[i] = residuals[i] <= threshold;
            }
            var support = 0; var squaredError = 0d;
            foreach (var residual in residuals)
                if (double.IsFinite(residual) && residual <= 6) { support++; squaredError += residual * residual; }
            accepted = support >= count * .95 && squaredError <= support * 9;
            return accepted ? coefficients : null;
        }
        finally
        {
            Array.Clear(samples); Array.Clear(residuals); Array.Clear(sorted); Array.Clear(included);
            if (!accepted) Array.Clear(coefficients);
        }
    }

    private static bool IsBoundary(byte[] state, int width, int height, int x, int y)
    {
        var index = y * width + x;
        return state[index] == Original && (x > 0 && state[index - 1] == Unknown ||
            x + 1 < width && state[index + 1] == Unknown || y > 0 && state[index - width] == Unknown ||
            y + 1 < height && state[index + width] == Unknown);
    }

    // Independent fast-marching reconstruction: advance from known boundaries,
    // weight nearby accepted colors and continue their local gradients. This is
    // inspired by Telea's method, not an OpenCV port or a generative image model.
    private static void RepairFromBoundary(byte[] pixels, byte[] state, int width, int height, int missing, CancellationToken token)
    {
        var distance = new float[state.Length];
        var queue = new PriorityQueue<int, (float Distance, int Index)>();
        try
        {
            for (var index = 0; index < state.Length; index++)
                if (state[index] == Unknown) distance[index] = float.PositiveInfinity;
            for (var y = 0; y < height; y++)
            {
                token.ThrowIfCancellationRequested();
                for (var x = 0; x < width; x++)
                    if (state[y * width + x] == Unknown) Offer(x, y);
            }
            var completed = 0;
            while (queue.TryDequeue(out var index, out var priority))
            {
                if ((completed & 255) == 0) token.ThrowIfCancellationRequested();
                if (state[index] == Repaired || distance[index] != priority.Distance) continue;
                var x = index % width; var y = index / width;
                RepairPixel(pixels, state, distance, width, height, x, y);
                state[index] = Repaired; completed++;
                if (x > 0) Offer(x - 1, y);
                if (x + 1 < width) Offer(x + 1, y);
                if (y > 0) Offer(x, y - 1);
                if (y + 1 < height) Offer(x, y + 1);
            }
            if (completed != missing) throw new InvalidOperationException(MissingBackgroundMessage);
        }
        finally { Array.Clear(distance); queue.Clear(); }

        void Offer(int x, int y)
        {
            var index = y * width + x;
            if (IsAccepted(state[index])) return;
            var horizontal = Math.Min(KnownDistance(x - 1, y), KnownDistance(x + 1, y));
            var vertical = Math.Min(KnownDistance(x, y - 1), KnownDistance(x, y + 1));
            var lower = Math.Min(horizontal, vertical);
            if (!float.IsFinite(lower)) return;
            var difference = Math.Abs(horizontal - vertical);
            var next = !float.IsFinite(difference) || difference >= 1 ? lower + 1 :
                (horizontal + vertical + MathF.Sqrt(2 - difference * difference)) * .5f;
            if (next >= distance[index]) return;
            distance[index] = next; state[index] = Frontier; queue.Enqueue(index, (next, index));
        }
        float KnownDistance(int x, int y) => x >= 0 && x < width && y >= 0 && y < height &&
            IsAccepted(state[y * width + x]) ? distance[y * width + x] : float.PositiveInfinity;
    }

    private static void RepairPixel(byte[] pixels, byte[] state, float[] distance, int width, int height, int x, int y)
    {
        var index = y * width + x;
        var normalX = DistanceGradient(x - 1, y, x + 1, y);
        var normalY = DistanceGradient(x, y - 1, x, y + 1);
        Span<double> values = stackalloc double[4];
        values.Clear();
        var weightSum = 0d;
        foreach (var neighbour in Neighbours)
        {
            var sx = x + neighbour.X; var sy = y + neighbour.Y;
            if (sx < 0 || sx >= width || sy < 0 || sy >= height) continue;
            var donor = sy * width + sx;
            if (!IsAccepted(state[donor])) continue;
            var direction = .05 + Math.Abs(neighbour.X * normalX + neighbour.Y * normalY);
            var weight = neighbour.Weight * direction / (1 + Math.Abs(distance[index] - distance[donor]));
            if (state[donor] == Repaired) weight *= .75;
            for (var channel = 0; channel < 4; channel++)
            {
                var color = pixels[donor * 4 + channel];
                var gradientX = ColorGradient(sx - 1, sy, sx + 1, sy, color, channel);
                var gradientY = ColorGradient(sx, sy - 1, sx, sy + 1, color, channel);
                values[channel] += weight * Math.Clamp(color - gradientX * neighbour.X - gradientY * neighbour.Y, 0, 255);
            }
            weightSum += weight;
        }
        if (weightSum <= 0) throw new InvalidOperationException(MissingBackgroundMessage);
        for (var channel = 0; channel < 4; channel++) pixels[index * 4 + channel] = ToByte(values[channel] / weightSum);
        values.Clear();

        bool Accepted(int sx, int sy) => sx >= 0 && sx < width && sy >= 0 && sy < height && IsAccepted(state[sy * width + sx]);
        double ColorGradient(int ax, int ay, int bx, int by, byte center, int channel)
        {
            var a = Accepted(ax, ay); var b = Accepted(bx, by);
            return a && b ? (pixels[(by * width + bx) * 4 + channel] - pixels[(ay * width + ax) * 4 + channel]) * .5 :
                a ? center - pixels[(ay * width + ax) * 4 + channel] : b ? pixels[(by * width + bx) * 4 + channel] - center : 0;
        }
        double DistanceGradient(int ax, int ay, int bx, int by)
        {
            var a = ax >= 0 && ax < width && ay >= 0 && ay < height && float.IsFinite(distance[ay * width + ax]);
            var b = bx >= 0 && bx < width && by >= 0 && by < height && float.IsFinite(distance[by * width + bx]);
            return a && b ? (distance[by * width + bx] - distance[ay * width + ax]) * .5 :
                a ? distance[index] - distance[ay * width + ax] : b ? distance[by * width + bx] - distance[index] : 0;
        }
    }

    private static bool IsAccepted(byte state) => state is Original or Repaired;
    private static double Normalize(double position, int length) => (position - (length - 1d) / 2) / Math.Max(1, length / 2d);
    private static byte ToByte(double value) => (byte)Math.Clamp(Math.Round(value), 0, 255);
    private static Neighbour[] CreateNeighbours()
    {
        var result = new List<Neighbour>();
        for (var y = -3; y <= 3; y++)
            for (var x = -3; x <= 3; x++)
            {
                var squared = x * x + y * y;
                if (squared > 0 && squared <= 9) result.Add(new Neighbour(x, y, 1d / (squared * Math.Sqrt(squared))));
            }
        return result.ToArray();
    }
}
