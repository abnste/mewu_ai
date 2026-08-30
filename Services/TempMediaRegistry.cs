using System.Diagnostics;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Keeps temporary media alive while a recorder, preview, export, or pinned window is using it.
/// Paths are normalized once and compared case-insensitively because the application is Windows-only.
/// </summary>
internal sealed class TempMediaRegistry
{
    private readonly object _gate = new();
    private readonly Dictionary<string, int> _leases = new(StringComparer.OrdinalIgnoreCase);

    internal static TempMediaRegistry Shared { get; } = new();

    internal int ActiveLeaseCount
    {
        get
        {
            lock (_gate) return _leases.Values.Sum();
        }
    }

    internal int ActivePathCount
    {
        get
        {
            lock (_gate) return _leases.Count;
        }
    }

    internal TempMediaLease Acquire(string path)
    {
        var normalizedPath = Normalize(path);
        lock (_gate)
        {
            _leases.TryGetValue(normalizedPath, out var count);
            _leases[normalizedPath] = checked(count + 1);
        }

        return new TempMediaLease(this, normalizedPath);
    }

    internal TempMediaLease AcquireExistingFile(string path)
    {
        var normalizedPath = Normalize(path);
        lock (_gate)
        {
            if (!File.Exists(normalizedPath))
                throw new FileNotFoundException("临时媒体文件已不可用", normalizedPath);
            _leases.TryGetValue(normalizedPath, out var count);
            _leases[normalizedPath] = checked(count + 1);
        }

        return new TempMediaLease(this, normalizedPath);
    }

    internal bool IsLeased(string path)
    {
        var normalizedPath = Normalize(path);
        lock (_gate) return _leases.ContainsKey(normalizedPath);
    }

    internal IReadOnlyDictionary<string, int> Snapshot()
    {
        lock (_gate) return new Dictionary<string, int>(_leases, StringComparer.OrdinalIgnoreCase);
    }

    internal bool WaitForNoActiveLeases(TimeSpan timeout)
    {
        if (timeout < TimeSpan.Zero) throw new ArgumentOutOfRangeException(nameof(timeout));
        var started = Stopwatch.GetTimestamp();
        lock (_gate)
        {
            while (_leases.Count > 0)
            {
                var remaining = timeout - Stopwatch.GetElapsedTime(started);
                if (remaining <= TimeSpan.Zero) return false;
                Monitor.Wait(_gate, remaining);
            }

            return true;
        }
    }

    internal bool TryExecuteIfUnleased(string path, bool includeDescendants, Action operation)
    {
        ArgumentNullException.ThrowIfNull(operation);
        var normalizedPath = Normalize(path);
        lock (_gate)
        {
            if (_leases.Keys.Any(activePath =>
                    string.Equals(activePath, normalizedPath, StringComparison.OrdinalIgnoreCase) ||
                    includeDescendants && IsDescendant(activePath, normalizedPath)))
                return false;

            operation();
            return true;
        }
    }

    internal void Release(string normalizedPath)
    {
        lock (_gate)
        {
            if (!_leases.TryGetValue(normalizedPath, out var count)) return;
            if (count == 1) _leases.Remove(normalizedPath);
            else _leases[normalizedPath] = count - 1;
            Monitor.PulseAll(_gate);
        }
    }

    private static bool IsDescendant(string candidate, string directory)
    {
        var prefix = directory.EndsWith(Path.DirectorySeparatorChar) || directory.EndsWith(Path.AltDirectorySeparatorChar)
            ? directory
            : directory + Path.DirectorySeparatorChar;
        return candidate.StartsWith(prefix, StringComparison.OrdinalIgnoreCase);
    }

    private static string Normalize(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        return Path.TrimEndingDirectorySeparator(Path.GetFullPath(path));
    }

}

public sealed class TempMediaLease : IDisposable
{
    private TempMediaRegistry? _owner;

    internal TempMediaLease(TempMediaRegistry owner, string path)
    {
        _owner = owner;
        Path = path;
    }

    ~TempMediaLease()=>Release();

    public string Path { get; }

    public void Dispose()
    {
        Release();
        GC.SuppressFinalize(this);
    }

    private void Release()=>Interlocked.Exchange(ref _owner, null)?.Release(Path);
}
