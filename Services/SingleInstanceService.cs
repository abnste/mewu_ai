using System.IO.Pipes;
namespace mewu_ai_Assistant.Services;
public sealed class SingleInstanceService : IDisposable
{
    private const string Name="MewuAI-7F2171C4"; private readonly Mutex _mutex; private readonly CancellationTokenSource _stop=new();
    public bool IsPrimary { get; }
    public event Action? ActivationRequested;
    public SingleInstanceService() { _mutex=new Mutex(true,Name,out var created); IsPrimary=created; if(created)_=ListenAsync(); }
    public void SignalPrimary() { try { using var client=new NamedPipeClientStream(".",Name,PipeDirection.Out); client.Connect(800); client.WriteByte(1); } catch { } }
    private async Task ListenAsync()
    {
        while(!_stop.IsCancellationRequested)
        {
            try { await using var server=new NamedPipeServerStream(Name,PipeDirection.In,1,PipeTransmissionMode.Byte,PipeOptions.Asynchronous); await server.WaitForConnectionAsync(_stop.Token); if(server.ReadByte()>=0) ActivationRequested?.Invoke(); }
            catch(OperationCanceledException) { break; } catch { await Task.Delay(250); }
        }
    }
    public void Dispose() { _stop.Cancel(); if(IsPrimary)_mutex.ReleaseMutex(); _mutex.Dispose(); _stop.Dispose(); }
}
