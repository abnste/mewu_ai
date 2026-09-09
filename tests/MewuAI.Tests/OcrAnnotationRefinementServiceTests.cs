// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class OcrAnnotationRefinementServiceTests
{
    [Fact]
    public void ShrinksCoarseCalloutsToSemanticallyMatchingOcrLines()
    {
        var document=new OcrDocument("",[
            Line("ESP32 主程序",120,55,210,32),
            Line("屏幕显示文字 光照强度 在第 1 行",95,120,520,38),
            Line("屏幕显示文字 读取环境光强度 在第 2 行",110,220,650,38),
            Line("如果 读取环境光强度 < 100",170,310,520,38),
            Line("灯号 0 蓝色",180,405,240,38),
            Line("关闭 0 号 LED",170,560,250,38)
        ]);
        var annotations=new[]
        {
            Note(.32,0,.62,.27,"① 程序入口：ESP32 主程序"),
            Note(.32,.20,.62,.20,"② 第 1 行显示「光照强度」"),
            Note(.26,.30,.65,.19,"③ 第 2 行实时读取光强度数值"),
            Note(.38,.39,.22,.08,"④ 阈值判断：光强度 < 100 = 暗"),
            Note(.34,.48,.30,.08,"⑤ 暗 → 显示蓝色"),
            Note(.37,.65,.31,.08,"⑥ 亮 → 关闭 0 号 LED")
        };

        var refined=OcrAnnotationRefinementService.RefineAll(document,1000,750,annotations,out var count);

        Assert.True(count==6,$"未修正索引：{string.Join(',',annotations.Select((note,index)=>(note,index)).Where(entry=>Equals(entry.note,refined[entry.index])).Select(entry=>entry.index))}");
        Assert.All(refined,note=>Assert.InRange(note.Height,.035,.08));
        Assert.InRange(refined[0].X,.10,.13);
        Assert.InRange(refined[1].Y,.14,.18);
        Assert.InRange(refined[2].Y,.27,.31);
        Assert.InRange(refined[3].Y,.39,.44);
        Assert.InRange(refined[4].Y,.52,.57);
        Assert.InRange(refined[5].Y,.72,.77);
    }

    [Fact]
    public void KeepsModelGeometryWhenOcrHasNoStrongSemanticMatch()
    {
        var original=Note(.2,.3,.4,.2,"圈出猫咪");
        var document=new OcrDocument("保存 取消",[Line("保存",20,20,50,20),Line("取消",80,20,50,20)]);

        var refined=Assert.Single(OcrAnnotationRefinementService.RefineAll(document,300,200,[original],out var count));

        Assert.Equal(0,count);
        Assert.Equal(original,refined);
    }

    [Fact]
    public void GradingExplanationCannotPullNegativeAnswerOntoQuestion()
    {
        var document=new OcrDocument("",[Line("25. 求 (-5)^-1 的值",30,200,210,28),Line("−5",495,250,35,31)]);
        var original=Note(.48,.32,.07,.07,"「-5」✗ 应为 −1/5：负指数取倒数，求 (-5)^-1 的值");
        var refined=Assert.Single(OcrAnnotationRefinementService.RefineAll(document,1010,740,[original],out var count));
        Assert.Equal(1,count);Assert.InRange(refined.X,.47,.5);Assert.InRange(refined.Y,.32,.35);
    }

    [Fact]
    public void MissingShortAnswerOcrDoesNotSnapToDistantMatchingQuestion()
    {
        var original=Note(.48,.32,.07,.07,"「-5」✗ 应为 −1/5：负指数取倒数");
        var document=new OcrDocument("",[Line("25. 求 (-5)^-1 的值",30,200,210,28)]);
        Assert.Equal(original,Assert.Single(OcrAnnotationRefinementService.RefineAll(document,1010,740,[original],out var count)));
        Assert.Equal(0,count);
    }

    [Fact]
    public void QuotedSingleDigitUsesNearbyAnswerRatherThanAnotherStudentsDigit()
    {
        var original=Note(.48,.32,.07,.07,"「2」正确");
        var document=new OcrDocument("",[Line("2",40,250,20,30),Line("2",495,250,20,30)]);
        var refined=Assert.Single(OcrAnnotationRefinementService.RefineAll(document,1010,740,[original],out var count));
        Assert.Equal(1,count);Assert.InRange(refined.X,.48,.5);
    }

    [Fact]
    public void UnquotedCorrectionCannotMoveAnAnswerMarkToTheQuestionColumn()
    {
        var original=Note(.47,.87,.16,.07,"4x²+1 错：完全平方缺中间项，应为 4x²+4x+1");
        var document=new OcrDocument("",[Line("29. 展开 (2x+1)²",30,610,230,30),Line("4x^2 + 1",495,650,145,34)]);
        var refined=Assert.Single(OcrAnnotationRefinementService.RefineAll(document,1010,740,[original],out _));
        Assert.InRange(refined.X,.45,.52);Assert.InRange(refined.Y,.85,.9);
    }

    private static OcrLine Line(string text,double x,double y,double width,double height)=>new(text,x,y,width,height,[new OcrWord(text,x,y,width,height)]);
    private static AiAnnotation Note(double x,double y,double width,double height,string text)=>new(x,y,width,height,text,ReferenceHandle:"ref-image",Kind:AiAnnotationKind.Callout);
}
