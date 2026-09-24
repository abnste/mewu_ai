// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.InteropServices;

namespace mewu_ai_Assistant.Recording;

/// <summary>
/// Keeps the selected render endpoint clocked while system-audio recording is
/// active. Some drivers stop producing loopback packets in silence; the native
/// recorder then waits for audio and repeats old video frames. This shared-mode
/// stream renders only zero samples, without changing volume or taking exclusive
/// ownership. All COM objects and GetBuffer/ReleaseBuffer pairs stay on one worker.
/// </summary>
internal sealed class LoopbackSilenceSource : IAsyncDisposable
{
    private readonly CancellationTokenSource _stop=new();
    private readonly TaskCompletionSource _ready=new(TaskCreationOptions.RunContinuationsAsynchronously);
    private readonly Task _worker;
    private int _disposed;
    private Exception? _failure;

    internal LoopbackSilenceSource(string? deviceId,Action<Exception> failed)
    {
        _worker=Task.Run(()=>Run(deviceId,failed));
    }

    internal void WaitUntilReady()=>_ready.Task.WaitAsync(TimeSpan.FromSeconds(5)).GetAwaiter().GetResult();
    internal Exception? Failure=>Volatile.Read(ref _failure);

    private void Run(string? deviceId,Action<Exception> failed)
    {
        IMMDeviceEnumerator? enumerator=null;IMMDevice? device=null;IAudioClient? client=null;IAudioRenderClient? render=null;
        var format=IntPtr.Zero;var started=false;
        try
        {
            enumerator=(IMMDeviceEnumerator)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("BCDE0395-E52F-467C-8E3D-C4579291692E"),true)!)!;
            Check(string.IsNullOrEmpty(deviceId)?enumerator.GetDefaultAudioEndpoint(0,0,out device):enumerator.GetDevice(deviceId,out device));
            var clientId=typeof(IAudioClient).GUID;Check(device.Activate(ref clientId,23,IntPtr.Zero,out var activated));client=(IAudioClient)activated;
            Check(client.GetMixFormat(out format));
            var blockAlign=(ushort)Marshal.ReadInt16(format,12);var bitsPerSample=(ushort)Marshal.ReadInt16(format,14);
            // AUDCLNT_SHAREMODE_SHARED and AUDCLNT_STREAMFLAGS_NOPERSIST.
            // No session volume, endpoint volume, or communications ducking is changed.
            Check(client.Initialize(0,0x00080000,2_000_000,0,format,IntPtr.Zero));
            Check(client.GetBufferSize(out var capacity));
            var byteCount=checked((int)(capacity*blockAlign));
            if(byteCount<=0||byteCount>4*1024*1024)throw new InvalidDataException("声音设备缓冲区大小无效");
            var zeros=new byte[byteCount];if(bitsPerSample==8)Array.Fill(zeros,(byte)128);
            var renderId=typeof(IAudioRenderClient).GUID;Check(client.GetService(ref renderId,out var service));render=(IAudioRenderClient)service;
            void Fill(uint frames)
            {
                if(frames==0)return;
                Check(render.GetBuffer(frames,out var target));
                try{Marshal.Copy(zeros,0,target,checked((int)(frames*blockAlign)));}
                finally{Check(render.ReleaseBuffer(frames,0));}
            }
            _stop.Token.ThrowIfCancellationRequested();Fill(capacity);Check(client.Start());started=true;_ready.TrySetResult();
            while(!_stop.Token.WaitHandle.WaitOne(10))
            {
                Check(client.GetCurrentPadding(out var padding));
                if(padding>capacity)throw new InvalidDataException("声音设备缓冲区状态无效");
                Fill(capacity-padding);
            }
        }
        catch(OperationCanceledException)when(_stop.IsCancellationRequested){_ready.TrySetCanceled();}
        catch(Exception ex)
        {
            Volatile.Write(ref _failure,ex);
            if(!_ready.TrySetException(ex)&&!_stop.IsCancellationRequested)
                try{failed(ex);}catch{ /* Do not let a reporting callback strand COM resources. */ }
        }
        finally
        {
            if(started)try{client?.Stop();}catch{}
            Release(render);Release(client);Release(device);Release(enumerator);
            if(format!=IntPtr.Zero)Marshal.FreeCoTaskMem(format);
        }
    }

    public async ValueTask DisposeAsync()
    {
        if(Interlocked.Exchange(ref _disposed,1)!=0)return;
        _stop.Cancel();
        // A broken audio driver must not block the UI or the recording watchdog.
        // The worker remains the sole owner and releases its objects when it returns.
        try{await _worker.WaitAsync(TimeSpan.FromSeconds(2)).ConfigureAwait(false);}
        catch(TimeoutException){}
        finally
        {
            if(_worker.IsCompleted)_stop.Dispose();
            else _=_worker.ContinueWith(_=>_stop.Dispose(),CancellationToken.None,TaskContinuationOptions.ExecuteSynchronously,TaskScheduler.Default);
        }
    }
    private static void Check(int result)=>Marshal.ThrowExceptionForHR(result);
    private static void Release(object? value)
    {
        // Each object is owned only by this worker. Still release the remaining
        // objects if an invalidated driver fails while tearing down one of them.
        try{if(value is not null&&Marshal.IsComObject(value))Marshal.FinalReleaseComObject(value);}
        catch(COMException){}
    }

    [ComImport,Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDeviceEnumerator
    {
        [PreserveSig]int EnumAudioEndpoints(int flow,uint mask,out IntPtr collection);
        [PreserveSig]int GetDefaultAudioEndpoint(int flow,int role,out IMMDevice device);
        [PreserveSig]int GetDevice([MarshalAs(UnmanagedType.LPWStr)]string id,out IMMDevice device);
    }
    [ComImport,Guid("D666063F-1587-4E43-81F1-B948E807363F"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDevice
    {
        [PreserveSig]int Activate(ref Guid id,uint context,IntPtr activation,[MarshalAs(UnmanagedType.IUnknown)]out object instance);
    }
    [ComImport,Guid("1CB9AD4C-DBFA-4C32-B178-C2F568A703B2"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IAudioClient
    {
        [PreserveSig]int Initialize(int mode,uint flags,long duration,long periodicity,IntPtr format,IntPtr session);
        [PreserveSig]int GetBufferSize(out uint frames);
        [PreserveSig]int GetStreamLatency(out long latency);
        [PreserveSig]int GetCurrentPadding(out uint frames);
        [PreserveSig]int IsFormatSupported(int mode,IntPtr format,out IntPtr closest);
        [PreserveSig]int GetMixFormat(out IntPtr format);
        [PreserveSig]int GetDevicePeriod(out long defaultPeriod,out long minimumPeriod);
        [PreserveSig]int Start();
        [PreserveSig]int Stop();
        [PreserveSig]int Reset();
        [PreserveSig]int SetEventHandle(IntPtr handle);
        [PreserveSig]int GetService(ref Guid id,[MarshalAs(UnmanagedType.IUnknown)]out object service);
    }
    [ComImport,Guid("F294ACFC-3146-4483-A7BF-ADDCA7C260E2"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IAudioRenderClient
    {
        [PreserveSig]int GetBuffer(uint frames,out IntPtr data);
        [PreserveSig]int ReleaseBuffer(uint frames,uint flags);
    }
}
