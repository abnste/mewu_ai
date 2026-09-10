// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.ExceptionServices;
using System.Threading;
using System.Windows;
using System.Windows.Documents;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

[Collection("Emoji WPF")]
public sealed class MarkdownFlowDocumentRendererTests
{
    [Theory]
    [InlineData(false)][InlineData(true)]
    public void PaperFeedbackAvoidsAnswersAndCrowdedInkAndExportPreservesOriginal(bool crowded)
    {
        RunSta(()=>
        {
            const int width=800,height=1100;
            var pixels=Enumerable.Repeat(crowded?(byte)0:(byte)255,width*height*4).ToArray();for(var p=3;p<pixels.Length;p+=4)pixels[p]=255;
            var image=System.Windows.Media.Imaging.BitmapSource.Create(width,height,96,96,System.Windows.Media.PixelFormats.Bgra32,null,pixels,width*4);image.Freeze();
            var item=new GradingItem("1","x¹³/y¹⁷","x¹¹/y¹⁷",GradingVerdict.Incorrect,"12−1 is 11.","Laws of indices",.15,.2,.28,.2){Confirmed=true};
            var page=new TeachingPage("paper","A",1,image,"public-fixture"){Items=[item]};var notes=TeachingSession.Annotations(page,"handle");
            var text=Assert.Single(notes,n=>n.Kind==mewu_ai_Assistant.Models.AiAnnotationKind.Text);
            Assert.Equal("handle",text.ReferenceHandle);Assert.InRange(text.X,0,1-text.Width);Assert.InRange(text.Y,0,1-text.Height);
            Assert.False(new Rect(item.X,item.Y,item.Width,item.Height).IntersectsWith(new Rect(text.X,text.Y,text.Width,text.Height)));
            if(crowded)Assert.DoesNotContain(item.Reason,text.Text);else{Assert.Contains(item.Reason,text.Text);Assert.Contains(item.Expected,text.Text);}
            var composite=mewu_ai_Assistant.Recording.AnnotationOverlayRenderer.ApplyAiAnnotations(image,notes);
            var original=new byte[pixels.Length];image.CopyPixels(original,width*4,0);Assert.Equal(pixels,original);
            var rendered=new byte[pixels.Length];composite.CopyPixels(rendered,width*4,0);Assert.False(rendered.SequenceEqual(original));
        });
    }
    [Fact]
    public void FormulasRenderAsVectorsAndCopyPreservesLatexAcrossStreaming()
    {
        RunSta(()=>
        {
            const string formula=@"\frac{x^{11}}{y^{17}}";
            var view=new mewu_ai_Assistant.Views.MarkdownAnswerView{Markdown="Answer: $"+formula+"$"};
            var paragraph=Assert.IsType<Paragraph>(view.Document.Blocks.FirstBlock);
            var inline=Assert.Single(paragraph.Inlines.OfType<InlineUIContainer>());
            var image=Assert.IsType<mewu_ai_Assistant.Views.MathFormulaView>(inline.Child);
            Assert.InRange(image.Height,20,100);Assert.InRange(image.Width,10,150);
            view.SelectAll();Assert.Contains("$"+formula+"$",view.SelectedPlainText);
            view.Markdown+="\n\n$$\n\\sqrt{x^2+1}\n$$";
            view.SelectAll();Assert.Contains(formula,view.SelectedPlainText);Assert.Contains(@"\sqrt{x^2+1}",view.SelectedPlainText);
            Assert.Equal(2,view.Document.Blocks.OfType<Paragraph>().SelectMany(p=>p.Inlines.OfType<InlineUIContainer>()).Count());
        });
    }
    [Fact]
    public void BracketMathAndAlignedStepsRenderWhileCodeKeepsLiteralDelimiters()
    {
        RunSta(()=>
        {
            const string inline=@"\(\frac{a}{b}\)";
            const string block=@"\[\begin{aligned} x+1 &= 2 \\ x &= 1 \end{aligned}\]";
            var document=MarkdownFlowDocumentRenderer.Render(inline+"\n\n"+block+"\n\n`"+inline+"`");
            var formulas=document.Blocks.OfType<Paragraph>().SelectMany(p=>p.Inlines.OfType<InlineUIContainer>()).ToArray();Assert.Equal(2,formulas.Length);
            var text=MarkdownFlowDocumentRenderer.ToPlainText(document);Assert.Contains(inline,text);Assert.Contains(block,text);
            Assert.Contains(document.Blocks.OfType<Paragraph>().Last().Inlines.OfType<Run>(),r=>r.Text==inline);
        });
    }
    [Fact]
    public void UnsupportedFormulaIsReadableAndDoesNotExecuteOrDisappear()
    {
        RunSta(()=>
        {
            const string input=@"$\input{private}$";
            var document=MarkdownFlowDocumentRenderer.Render(input);
            Assert.Contains(input,MarkdownFlowDocumentRenderer.ToPlainText(document));
            Assert.Null(MathFormulaRenderer.Create(input,18,System.Windows.Media.Brushes.Black));
            Assert.False(MathFormulaRenderer.IsBoundedFormula(new string('{',17)+"x"+new string('}',17)));
            Assert.Null(MathFormulaRenderer.Create("$"+new string('x',3000)+"$",18,System.Windows.Media.Brushes.Black));
        });
    }
    [Fact]
    public void ExamQuestionNumbersKeepTheirOrderedListStartInRenderingAndCopy()
    {
        RunSta(()=>
        {
            var view=new mewu_ai_Assistant.Views.MarkdownAnswerView{Markdown="24. 方程正确\n25. 负指数错误"};
            var list=Assert.Single(view.Document.Blocks.OfType<System.Windows.Documents.List>());
            Assert.Equal(24,list.StartIndex);
            view.SelectAll();Assert.Contains("24.",view.SelectedPlainText);Assert.Contains("25.",view.SelectedPlainText);
            view.Markdown+="\n26. 科学记数法正确";
            Assert.Equal(24,Assert.Single(view.Document.Blocks.OfType<System.Windows.Documents.List>()).StartIndex);
            view.SelectAll();Assert.Contains("26.",view.SelectedPlainText);
        });
    }

    [Fact]
    public void ChineseReplyUsesExplicitChineseFontAndKeepsOriginalCharactersWhenCopied()
    {
        RunSta(() =>
        {
            const string text="视频发生强烈闪光，随后扩散。原文：影片發生強烈閃光。 👋🏽";
            var view=new mewu_ai_Assistant.Views.MarkdownAnswerView{Markdown=text};
            Assert.Contains("Microsoft YaHei UI",view.Document.FontFamily.Source);
            Assert.Equal(LocalizationService.CultureName,view.Document.Language.IetfLanguageTag,StringComparer.OrdinalIgnoreCase);
            view.Markdown=text+"\n\n`变量 = \"原文\"`";
            view.SelectAll();
            Assert.Contains(text,view.SelectedPlainText);
            Assert.Contains("变量 = \"原文\"",view.SelectedPlainText);
            foreach(var block in view.Document.Blocks.OfType<Paragraph>())
                foreach(var run in block.Inlines.OfType<Run>().Where(run=>!MarkdownFlowDocumentRenderer.IsEmoji(run.Text)))
                    Assert.Contains("Microsoft YaHei UI",run.FontFamily.Source);
        });
    }

    [Fact]
    public void RendersCommonMarkdownAsSelectableDocumentText()
    {
        RunSta(() =>
        {
            var document=MarkdownFlowDocumentRenderer.Render("# 标题\n\n**加粗** 和 *斜体*\n\n- 第一项\n- 第二项\n\n```json\n{\"ok\":true}\n```");
            var text=MarkdownFlowDocumentRenderer.ToPlainText(document);

            Assert.Contains("标题",text);
            Assert.Contains("加粗 和 斜体",text);
            Assert.Contains("第一项",text);
            Assert.Contains("{\"ok\":true}",text);
            Assert.DoesNotContain("**",text);
            Assert.DoesNotContain("```",text);
        });
    }

    [Fact]
    public void PreservesUnicodeEmojiAndUsesEmojiRuns()
    {
        RunSta(() =>
        {
            var document=MarkdownFlowDocumentRenderer.Render("你好 👋🏽 🧑‍💻 🎉");
            Assert.Equal("你好 👋🏽 🧑‍💻 🎉",MarkdownFlowDocumentRenderer.ToPlainText(document));
            var paragraph=Assert.IsType<Paragraph>(document.Blocks.FirstBlock);
            Assert.Contains(paragraph.Inlines.OfType<Run>(),run=>run.FontFamily.Source=="Segoe UI Emoji");
            Assert.True(MarkdownFlowDocumentRenderer.IsEmoji("👋🏽"));
            Assert.False(MarkdownFlowDocumentRenderer.IsEmoji("中文"));
        });
    }

    [Fact]
    public void MarkdownAnswerViewConvertsEmojiToColorVectorInlinesAndPreservesCopiedText()
    {
        RunSta(() =>
        {
            var view=new mewu_ai_Assistant.Views.MarkdownAnswerView
            {
                Markdown="彩色 👋🏽 🧑‍💻 🇨🇳"
            };

            Assert.Equal(3,view.EmojiInlines.Count());
            Assert.Equal("彩色 👋🏽 🧑‍💻 🇨🇳",view.PlainText);
            Assert.All(view.EmojiInlines,emoji=>Assert.NotNull(emoji.Child.Source));
            view.SelectAll();Assert.Equal(view.PlainText,view.SelectedPlainText);
            var firstEmoji=view.EmojiInlines.First();
            view.Selection.Select(firstEmoji.ElementStart,firstEmoji.ElementEnd);
            Assert.Equal("👋🏽",view.SelectedPlainText);
        });
    }

    [Fact]
    public void DoesNotExecuteHtmlAndDefersImageLoadingUntilDisplay()
    {
        RunSta(() =>
        {
            var document=MarkdownFlowDocumentRenderer.Render("<script>alert('x')</script>\n\n![示意图](https://example.invalid/a.png)");
            var text=MarkdownFlowDocumentRenderer.ToPlainText(document);

            Assert.Contains("<script>alert('x')</script>",text);
            Assert.Contains(LocalizationService.IsEnglish?"[Image: 示意图]":"[图片：示意图]",text);
            Assert.DoesNotContain("[图片：",new TextRange(document.ContentStart,document.ContentEnd).Text);
            Assert.DoesNotContain("[Image:",new TextRange(document.ContentStart,document.ContentEnd).Text);
            var image=Assert.Single(document.Blocks.OfType<Paragraph>().SelectMany(paragraph=>paragraph.Inlines.OfType<InlineUIContainer>()));
            Assert.False(Assert.IsAssignableFrom<FrameworkElement>(image.Child).IsLoaded);
        });
    }

    [Fact]
    public void ImageDescriptionIsCopyOnlyAndKeepsSurroundingEmojiOrder()
    {
        RunSta(() =>
        {
            var view=new mewu_ai_Assistant.Views.MarkdownAnswerView{Markdown="前 👋 ![画像](https://example.invalid/a.png) 后 🎉"};
            Assert.Equal(LocalizationService.IsEnglish?"前 👋 [Image: 画像] 后 🎉":"前 👋 [图片：画像] 后 🎉",view.PlainText);
            view.SelectAll();Assert.Equal(view.PlainText,view.SelectedPlainText);
            Assert.DoesNotContain("[图片：",new TextRange(view.Document.ContentStart,view.Document.ContentEnd).Text);
            Assert.DoesNotContain("[Image:",new TextRange(view.Document.ContentStart,view.Document.ContentEnd).Text);
        });
    }

    [Fact]
    public void ActivatesOnlyHttpAndHttpsLinks()
    {
        RunSta(() =>
        {
            var document=MarkdownFlowDocumentRenderer.Render("[网页](https://example.com) [危险](file:///C:/Windows/win.ini)");
            var paragraph=Assert.IsType<Paragraph>(document.Blocks.FirstBlock);
            var links=paragraph.Inlines.OfType<Hyperlink>().ToArray();

            Assert.Equal(2,links.Length);
            Assert.Equal("https",links[0].NavigateUri?.Scheme);
            Assert.Null(links[1].NavigateUri);
        });
    }

    [Fact]
    public void MarkdownAnswerViewAppendsClickableLocalVideoActions()
    {
        RunSta(() =>
        {
            var oldInvoked=false;var currentInvoked=false;var view=new mewu_ai_Assistant.Views.MarkdownAnswerView();
            view.SetMarkdownWithActions("错误发生在按钮点击后",[new mewu_ai_Assistant.Views.MarkdownAnswerAction("12秒：按钮","旧标记",()=>oldInvoked=true)]);
            view.SetMarkdownWithActions("错误发生在按钮点击后",[new mewu_ai_Assistant.Views.MarkdownAnswerAction("18秒：黑色手机","当前标记",()=>currentInvoked=true)]);
            var container=Assert.IsType<BlockUIContainer>(view.Document.Blocks.LastBlock);var panel=Assert.IsType<System.Windows.Controls.WrapPanel>(container.Child);var chip=Assert.IsType<System.Windows.Controls.Button>(Assert.Single(panel.Children));
            Assert.DoesNotContain("视频定位",MarkdownFlowDocumentRenderer.ToPlainText(view.Document));Assert.Equal("18秒：黑色手机",chip.Content);
            chip.RaiseEvent(new RoutedEventArgs(System.Windows.Controls.Button.ClickEvent));Assert.True(currentInvoked);Assert.False(oldInvoked);
        });
    }

    private static void RunSta(Action action)
    {
        Exception? failure=null;
        var thread=new Thread(() =>
        {
            try { action(); }
            catch(Exception ex) { failure=ex; }
        });
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        thread.Join();
        if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
