// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Reflection;
using System.Text.Json;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

[CollectionDefinition("Reference localization state", DisableParallelization = true)]
public sealed class ReferenceLocalizationStateCollection { }

[Collection("Reference localization state")]
public sealed class ReferenceLabelServiceTests
{
    [Theory]
    [InlineData(0, "@Image12", "@Video3", "@File42", "@CurrentScreen")]
    [InlineData(1, "@图片12", "@视频3", "@文件42", "@当前屏幕")]
    public void LabelsFollowUiLanguageWithoutChangingTheirNumbers(int language, string image, string video, string file, string screen)
        => WithLanguage((AppLanguage)language, () =>
        {
            Assert.Equal(image, ReferenceLabelService.ForSelection(false, 12));
            Assert.Equal(video, ReferenceLabelService.ForSelection(true, 3));
            Assert.Equal(file, ReferenceLabelService.ForUpload(42));
            Assert.Equal(screen, ReferenceLabelService.CurrentScreen);
        });

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    public void ReferencePromptPreservesProtocolIdentityMetadataAndUserText(int language)
        => WithLanguage((AppLanguage)language, () =>
        {
            AttachmentReferenceDescriptor[] references =
            [
                new(0, "capture-image-stable", ReferenceLabelService.ForSelection(false, 12), AiAttachmentType.Image, 640, 480, null, true, true),
                new(1, "capture-video-stable", ReferenceLabelService.ForSelection(true, 3), AiAttachmentType.Video, 1280, 720, 12.75, true),
                new(2, "upload-text-stable", ReferenceLabelService.ForUpload(42), AiAttachmentType.Text, 0, 0, null, false),
                new(3, "capture-screen-stable", ReferenceLabelService.CurrentScreen, AiAttachmentType.Image, 2560, 1440, null, true)
            ];
            var unchanged = references.ToArray();
            var userText = "  Compare " + references[0].Label + " and " + references[1].Label +
                ".\n原文 @图片9 / @文件1 must stay unchanged: \"A&B <tag>\" 😀\r\n  ";
            var prompt = CaptureOverlayPolicy.CreateReferenceAwarePrompt(userText, references);
            var questionPrefix = LocalizationService.IsEnglish ? "\nUser question: " : "\n用户问题：";
            const string manifestPrefix = "\nattachmentReferences=";
            var manifestStart = prompt.IndexOf(manifestPrefix, StringComparison.Ordinal) + manifestPrefix.Length;
            Assert.True(manifestStart >= manifestPrefix.Length);
            var questionStart = prompt.IndexOf(questionPrefix, manifestStart, StringComparison.Ordinal);
            Assert.True(questionStart > manifestStart);
            Assert.Equal(userText, prompt[(questionStart + questionPrefix.Length)..]);
            Assert.Equal(unchanged, references);
            Assert.Contains("mewu.visual-annotations/1", prompt);
            Assert.Contains(LocalizationService.IsEnglish ? "never infer regionIndex from the display number" : "禁止按显示编号猜测 regionIndex", prompt);

            using var manifest = JsonDocument.Parse(prompt[manifestStart..questionStart]);
            var items = manifest.RootElement.EnumerateArray().ToArray();
            Assert.Equal(references.Length, items.Length);
            string[] types = ["image", "video", "text", "image"];
            for (var i = 0; i < references.Length; i++)
            {
                var expected = references[i];
                var actual = items[i];
                Assert.Equal(i, actual.GetProperty("RegionIndex").GetInt32());
                Assert.Equal(expected.ReferenceHandle, actual.GetProperty("ReferenceHandle").GetString());
                Assert.Equal(expected.Label, actual.GetProperty("Label").GetString());
                Assert.Equal(types[i], actual.GetProperty("type").GetString());
                Assert.Equal(expected.PixelWidth, actual.GetProperty("pixelWidth").GetInt32());
                Assert.Equal(expected.PixelHeight, actual.GetProperty("pixelHeight").GetInt32());
                Assert.Equal(expected.CanRenderAnnotations, actual.GetProperty("canRenderAnnotations").GetBoolean());
                Assert.Equal(expected.HasExistingAiAnnotations, actual.GetProperty("hasExistingAiAnnotations").GetBoolean());
                if (expected.DurationSeconds is { } duration) Assert.Equal(duration, actual.GetProperty("durationSeconds").GetDouble());
                else Assert.Equal(JsonValueKind.Null, actual.GetProperty("durationSeconds").ValueKind);
                var handles = actual.GetProperty("coordinateHandles");
                Assert.Equal(new[] { 0, 0 }, handles.GetProperty("topLeft").EnumerateArray().Select(value => value.GetInt32()));
                Assert.Equal(new[] { 1, 0 }, handles.GetProperty("topRight").EnumerateArray().Select(value => value.GetInt32()));
                Assert.Equal(new[] { 0, 1 }, handles.GetProperty("bottomLeft").EnumerateArray().Select(value => value.GetInt32()));
                Assert.Equal(new[] { 1, 1 }, handles.GetProperty("bottomRight").EnumerateArray().Select(value => value.GetInt32()));
            }
        });

    private static void WithLanguage(AppLanguage language, Action check)
    {
        // Do not call Initialize: these string-only tests neither create WPF
        // metadata nor read settings. This collection is isolated from parallel
        // tests, and the process-wide language is restored even on assertion failure.
        var property = typeof(LocalizationService).GetProperty(nameof(LocalizationService.Language),
            BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic)!;
        var previous = LocalizationService.Language;
        try { property.SetValue(null, language); check(); }
        finally { property.SetValue(null, previous); }
    }
}
