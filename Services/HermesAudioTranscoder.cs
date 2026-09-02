using System.Buffers;
using System.Buffers.Binary;
using System.Security.Cryptography;
using System.Text;
using Concentus;
using Concentus.Oggfile;
using Concentus.Structs;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Converts the Ogg/Opus payload emitted by some Hermes TTS providers to a
/// PCM WAV payload. Windows Media Foundation does not include an Ogg decoder
/// on every supported Windows installation, while WAV playback is universally
/// available. The conversion is entirely in-process and keeps the provider's
/// cloned voice intact.
/// </summary>
internal static class HermesAudioTranscoder
{
    private const int SampleRate=48_000;
    private const int MaxChannels=2;
    private const long MaxDecodedPcmBytes=128L*1024*1024;
    private const int MaxPackets=200_000;
    private static readonly byte[] OpusHead=Encoding.ASCII.GetBytes("OpusHead");

    internal static bool IsOggOpus(string mimeType,string extension,ReadOnlySpan<byte> data)
    {
        if(!string.Equals(mimeType,"audio/ogg",StringComparison.OrdinalIgnoreCase)&&
           !string.Equals(mimeType,"audio/opus",StringComparison.OrdinalIgnoreCase)&&
           !string.Equals(extension,".ogg",StringComparison.OrdinalIgnoreCase)&&
           !string.Equals(extension,".opus",StringComparison.OrdinalIgnoreCase))return false;
        return FindOpusHead(data)>=0;
    }

    internal static byte[] DecodeToWave(byte[] data)
    {
        ArgumentNullException.ThrowIfNull(data);
        return DecodeToWaveCore(data,new MemoryStream(data,writable:false));
    }

    internal static byte[] DecodeToWave(ReadOnlySpan<byte> data)
    {
        var copy=data.ToArray();
        try{return DecodeToWave(copy);}
        finally{CryptographicOperations.ZeroMemory(copy);}
    }

    private static byte[] DecodeToWaveCore(ReadOnlySpan<byte> data,MemoryStream input)
    {
        var headerOffset=FindOpusHead(data);
        if(headerOffset<0||headerOffset+19>data.Length)
            throw new InvalidDataException("Hermes 返回的 Ogg 音频不是有效的 Opus 流。");

        var channels=data[headerOffset+9];
        if(channels is <1 or >MaxChannels)
            throw new InvalidDataException("Hermes 返回的 Opus 声道数不受支持。");
        var preSkip=BinaryPrimitives.ReadUInt16LittleEndian(data[(headerOffset+10)..]);

        var decoder=OpusCodecFactory.CreateDecoder(SampleRate,channels);
        using var inputStream=input;
        var reader=new OpusOggReadStream(decoder,inputStream);
        using var output=new MemoryStream(capacity:Math.Min((int)MaxDecodedPcmBytes,Math.Max(44,data.Length*4)));
        output.SetLength(44);
        long pcmBytes=0;
        var skippedSamples=(int)preSkip;
        var packetCount=0;
        try
        {
            try
            {
                while(reader.HasNextPacket)
                {
                    if(++packetCount>MaxPackets)throw new InvalidDataException("Hermes 返回的朗读音频过长。");
                    var samples=reader.DecodeNextPacket();
                    if(samples is null)
                    {
                        if(!string.IsNullOrWhiteSpace(reader.LastError))
                            throw new InvalidDataException("Hermes 返回的 Opus 音频无法解码。");
                        break;
                    }
                    try
                    {
                        if(samples.Length%channels!=0)throw new InvalidDataException("Hermes 返回的 Opus 音频帧无效。");
                        var frameSamples=samples.Length/channels;
                        var start=Math.Min(skippedSamples,frameSamples);
                        skippedSamples-=start;
                        var samplesToWrite=frameSamples-start;
                        var frameBytes=checked(samplesToWrite*channels*2L);
                        if(frameBytes>0)
                        {
                            if(pcmBytes>MaxDecodedPcmBytes-frameBytes)throw new InvalidDataException("Hermes 解码后的朗读数据超过安全上限。");
                            var encoded=ArrayPool<byte>.Shared.Rent(checked((int)frameBytes));
                            try
                            {
                                var outputOffset=0;
                                for(var index=start*channels;index<samples.Length;index++)
                                {
                                    BinaryPrimitives.WriteInt16LittleEndian(encoded.AsSpan(outputOffset,2),samples[index]);
                                    outputOffset+=2;
                                }
                                output.Write(encoded,0,outputOffset);
                                pcmBytes+=outputOffset;
                            }
                            finally
                            {
                                CryptographicOperations.ZeroMemory(encoded.AsSpan(0,checked((int)frameBytes)));
                                ArrayPool<byte>.Shared.Return(encoded);
                            }
                        }
                    }
                    finally
                    {
                        Array.Clear(samples);
                    }
                }
            }
            finally{reader.Close();}

            if(pcmBytes<=0)throw new InvalidDataException("Hermes 返回的 Opus 音频没有可播放内容。");
            WriteWaveHeader(output,channels,pcmBytes);
            return output.ToArray();
        }
        finally
        {
            if(output.TryGetBuffer(out var outputBuffer))
                CryptographicOperations.ZeroMemory(outputBuffer.AsSpan(0,checked((int)output.Length)));
        }
    }

    private static int FindOpusHead(ReadOnlySpan<byte> data)
    {
        for(var index=0;index<=data.Length-OpusHead.Length;index++)
        {
            if(data[index..(index+OpusHead.Length)].SequenceEqual(OpusHead))return index;
        }
        return -1;
    }

    private static void WriteWaveHeader(MemoryStream output,int channels,long pcmBytes)
    {
        if(pcmBytes>uint.MaxValue-36)throw new InvalidDataException("Hermes WAV 数据超过格式上限。");
        var buffer=output.GetBuffer().AsSpan(0,44);
        buffer.Clear();
        Encoding.ASCII.GetBytes("RIFF").CopyTo(buffer);
        BinaryPrimitives.WriteUInt32LittleEndian(buffer[4..],checked((uint)(36+pcmBytes)));
        Encoding.ASCII.GetBytes("WAVEfmt ").CopyTo(buffer[8..]);
        BinaryPrimitives.WriteUInt32LittleEndian(buffer[16..],16);
        BinaryPrimitives.WriteUInt16LittleEndian(buffer[20..],1);
        BinaryPrimitives.WriteUInt16LittleEndian(buffer[22..],checked((ushort)channels));
        BinaryPrimitives.WriteUInt32LittleEndian(buffer[24..],SampleRate);
        BinaryPrimitives.WriteUInt32LittleEndian(buffer[28..],checked((uint)(SampleRate*channels*2)));
        BinaryPrimitives.WriteUInt16LittleEndian(buffer[32..],checked((ushort)(channels*2)));
        BinaryPrimitives.WriteUInt16LittleEndian(buffer[34..],16);
        Encoding.ASCII.GetBytes("data").CopyTo(buffer[36..]);
        BinaryPrimitives.WriteUInt32LittleEndian(buffer[40..],checked((uint)pcmBytes));
    }
}
