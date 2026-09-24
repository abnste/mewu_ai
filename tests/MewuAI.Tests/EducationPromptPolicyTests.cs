// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class EducationPromptPolicyTests
{
    [Fact]
    public void LearningGuidanceIsNaturalLanguageAndCoversBroadLearningTasks()
    {
        var instruction=EducationPromptPolicy.Instruction;
        Assert.Contains("帮我批改这张试卷",instruction);
        Assert.Contains("图形化编程",instruction);
        Assert.Contains("Python",instruction);
        Assert.Contains("任何知识",instruction);
        Assert.Contains("填答案",instruction);
        Assert.Contains("解释",instruction);
        Assert.Contains("LaTeX",instruction);
        Assert.Contains("代码块",instruction);
        Assert.Contains("逐字符",instruction);
        Assert.Contains("错误的缩进",instruction);
        Assert.Contains("待核",instruction);
        Assert.DoesNotContain("教学面板",instruction);
    }

    [Theory]
        [InlineData("帮我批改这篇语文阅读，指出第2题的错误")]
        [InlineData("帮我批改这张数学试卷，指出第2题的错误")]
    [InlineData("请用大白话解释这段英语阅读")]
    [InlineData("把 Scratch 角色移动代码改正确")]
    [InlineData("检查这段 Python 并给出可运行修正版")]
    [InlineData("根据截图备一节物理课")]
    public void ReferenceAwareLearningPromptKeepsOneConversationContract(string userPrompt)
    {
        var prompt=CaptureOverlayPolicy.CreateReferenceAwarePrompt(userPrompt,[
            new(0,"capture-1","@图片1",AiAttachmentType.Image,1200,800,null,true,false)
        ]);

        Assert.Contains(VisualAnnotationProtocol.Version,prompt);
        Assert.Contains(EducationPromptPolicy.Instruction,prompt);
        Assert.Contains(userPrompt,prompt);
        Assert.DoesNotContain("教学面板",prompt);
        Assert.DoesNotContain("侧栏",prompt);
    }
}
