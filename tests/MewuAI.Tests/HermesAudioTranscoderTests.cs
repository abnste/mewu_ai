using System.Buffers.Binary;
using Concentus;
using Concentus.Enums;
using Concentus.Oggfile;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class HermesAudioTranscoderTests
{
    [Fact]
    public void OggOpusIsDecodedToPlayablePcmWave()
    {
        using var encodedStream = new MemoryStream();
        var encoder = OpusCodecFactory.CreateEncoder(48_000, 1, OpusApplication.OPUS_APPLICATION_AUDIO);
        var tags = new OpusTags();
        var samples = new short[4_800];
        for (var index = 0; index < samples.Length; index++)
            samples[index] = (short)(Math.Sin(index * 0.08) * 12_000);
        var writer = new OpusOggWriteStream(encoder, encodedStream, tags, inputSampleRate: 48_000);
        writer.WriteSamples(samples, 0, samples.Length);
        writer.Finish();

        var ogg = encodedStream.ToArray();
        Assert.True(HermesAudioTranscoder.IsOggOpus("audio/ogg", ".ogg", ogg));
        var wave = HermesAudioTranscoder.DecodeToWave(ogg);

        Assert.Equal("RIFF", System.Text.Encoding.ASCII.GetString(wave, 0, 4));
        Assert.Equal("WAVE", System.Text.Encoding.ASCII.GetString(wave, 8, 4));
        Assert.Equal((ushort)1, BinaryPrimitives.ReadUInt16LittleEndian(wave.AsSpan(22, 2)));
        Assert.Equal((uint)48_000, BinaryPrimitives.ReadUInt32LittleEndian(wave.AsSpan(24, 4)));
        Assert.True(BinaryPrimitives.ReadUInt32LittleEndian(wave.AsSpan(40, 4)) > 0);
    }

    [Theory]
    [InlineData("audio/ogg", ".ogg", true)]
    [InlineData("audio/mpeg", ".mp3", false)]
    [InlineData("audio/ogg", ".ogg", false)]
    public void OggOpusDetectionRequiresOpusHeader(string mime, string extension, bool expected)
    {
        var payload = expected
            ? System.Text.Encoding.ASCII.GetBytes("OggS........OpusHead\u0001\u0001\u0000\u0000\u0000\u0000\u0000\u0000\u0000\u0000\u0000")
            : new byte[] { 0x4f, 0x67, 0x67, 0x53 };
        Assert.Equal(expected, HermesAudioTranscoder.IsOggOpus(mime, extension, payload));
    }
}
