using System.Runtime.ExceptionServices;
using System.Threading;
using System.Windows.Documents;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class MarkdownFlowDocumentRendererTests
{
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
    public void DoesNotExecuteHtmlOrLoadRemoteImages()
    {
        RunSta(() =>
        {
            var document=MarkdownFlowDocumentRenderer.Render("<script>alert('x')</script>\n\n![示意图](https://example.invalid/a.png)");
            var text=MarkdownFlowDocumentRenderer.ToPlainText(document);

            Assert.Contains("<script>alert('x')</script>",text);
            Assert.Contains("[图片：示意图]",text);
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
