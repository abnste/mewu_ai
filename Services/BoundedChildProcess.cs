// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.Text;

namespace mewu_ai_Assistant.Services;

internal sealed record ChildProcessResult(int ExitCode,string Output,bool OutputTruncated,long ErrorCharacters);

/// <summary>Drain both pipes immediately; bound retained output and process lifetime.</summary>
internal static class BoundedChildProcess
{
    internal static async Task<ChildProcessResult> RunAsync(ProcessStartInfo start,TimeSpan budget,
        CancellationToken token,int maxOutputCharacters=4096)
    {
        token.ThrowIfCancellationRequested();
        start.UseShellExecute=false;start.CreateNoWindow=true;
        start.RedirectStandardOutput=true;start.RedirectStandardError=true;
        using var deadline=CancellationTokenSource.CreateLinkedTokenSource(token);
        deadline.CancelAfter(budget);
        using var process=Process.Start(start)??throw new InvalidOperationException("无法启动子进程 / Unable to start process.");
        var output=DrainAsync(process.StandardOutput,maxOutputCharacters,deadline.Token);
        var errors=DrainAsync(process.StandardError,0,deadline.Token);
        try
        {
            await process.WaitForExitAsync(deadline.Token).ConfigureAwait(false);
            var stdout=await output.ConfigureAwait(false);
            var stderr=await errors.ConfigureAwait(false);
            return new(process.ExitCode,stdout.Text,stdout.Count>maxOutputCharacters,stderr.Count);
        }
        catch(OperationCanceledException)when(!token.IsCancellationRequested)
        {
            throw new TimeoutException("子进程操作超时 / The process operation timed out.");
        }
        finally
        {
            if(!process.HasExited)
                try{process.Kill(entireProcessTree:true);}catch(Exception ex)when(ex is InvalidOperationException or System.ComponentModel.Win32Exception){}
            await deadline.CancelAsync().ConfigureAwait(false);
            try{await Task.WhenAll(output,errors,process.WaitForExitAsync()).WaitAsync(TimeSpan.FromSeconds(2)).ConfigureAwait(false);}
            catch(Exception ex)when(ex is OperationCanceledException or TimeoutException or IOException or InvalidOperationException){}
        }
    }

    private static async Task<(string Text,long Count)> DrainAsync(StreamReader reader,int limit,CancellationToken token)
    {
        var buffer=new char[4096];var text=new StringBuilder();long count=0;
        try
        {
            int read;
            while((read=await reader.ReadAsync(buffer.AsMemory(),token).ConfigureAwait(false))>0)
            {
                count+=read;
                var keep=Math.Min(read,Math.Max(0,limit-text.Length));
                if(keep>0)text.Append(buffer,0,keep);
                Array.Clear(buffer);
            }
            return(text.ToString(),count);
        }
        finally{Array.Clear(buffer);}
    }
}
