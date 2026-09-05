using System.Runtime.ExceptionServices;
using System.Windows.Documents;
using System.Windows.Threading;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Xunit;

namespace MewuAI.Tests;

[Collection("Emoji WPF")]
public sealed class IncrementalMarkdownRendererTests
{
    [Fact]
    public void AppendingKeepsSelectedEmojiTextAndParagraphBreaks()
    {
        RunSta(()=>
        {
            var view=new MarkdownAnswerView {Markdown="第一段 👋🏽\n\n正在"};
            var first=view.Document.Blocks.FirstBlock;
            ((System.Windows.Controls.RichTextBox)view).Selection.Select(first.ContentStart,first.ContentEnd);
            var selected=view.Selection.Text;
            view.Markdown="第一段 👋🏽\n\n正在更新 🇨🇳";
            Assert.Equal(selected,view.Selection.Text);
            Assert.Contains("👋🏽",view.Selection.Text);
            Assert.Contains("\r\n",view.PlainText);
            Assert.Contains("🇨🇳",view.PlainText);
            Assert.Equal(2,view.EmojiInlines.Count());
        });
    }

    [Fact]
    public void StreamingKeepsSettledParagraphAndDocumentAlive()
    {
        RunSta(()=>
        {
            var view=new MarkdownAnswerView {Markdown="**保留**这一段\n\n正在"};
            var document=view.Document;var first=document.Blocks.FirstBlock;
            view.Markdown="**保留**这一段\n\n正在继续回答";
            Assert.Same(document,view.Document);Assert.Same(first,view.Document.Blocks.FirstBlock);
            Assert.Contains("正在继续回答",view.PlainText);
            view.Markdown="全新回答";
            Assert.DoesNotContain("保留",view.PlainText);
        });
    }

    [Theory]
    [InlineData("段落\n\n标题\n", "段落\n\n标题\n---\n")]
    [InlineData("- 一\n", "- 一\n- 二\n\n尾部")]
    [InlineData("```csharp\nvar x", "```csharp\nvar x=1;\n```\n\n尾部")]
    [InlineData("[示例][id]\n", "[示例][id]\n\n[id]: https://example.com\n")]
    [InlineData("A | B\n", "A | B\n--- | ---\n1 | 2\n")]
    [InlineData("长段落\n\n第二段", "短")]
    [InlineData("内容", "")]
    public void UpdatedDocumentMatchesFreshMarkdownRender(string before,string after)
    {
        RunSta(()=>
        {
            var renderer=new IncrementalMarkdownRenderer();renderer.Update(before,13,out _);
            var updated=renderer.Update(after,13,out var containsTable);
            var expected=MarkdownFlowDocumentRenderer.Render(after,13,out var expectedTable);
            Assert.Equal(MarkdownFlowDocumentRenderer.ToPlainText(expected),MarkdownFlowDocumentRenderer.ToPlainText(updated));
            Assert.Equal(expectedTable,containsTable);
            if(after.Length>0)Assert.Equal(expected.Blocks.Select(block=>block.GetType()),updated.Blocks.Select(block=>block.GetType()));
            if(after.Contains("https://"))
                Assert.Contains(updated.Blocks.OfType<Paragraph>().SelectMany(p=>p.Inlines.OfType<Hyperlink>()),link=>link.NavigateUri?.Host=="example.com");
        });
    }

    [Fact]
    public void ReplacingActionsLeavesOnlyTheCurrentButtons()
    {
        RunSta(()=>
        {
            var view=new MarkdownAnswerView();
            view.SetMarkdownWithActions("正文",[new("旧动作","旧",()=>{})]);
            view.SetMarkdownWithActions("正文",[new("新动作","新",()=>{})]);
            Assert.Single(view.Document.Blocks.OfType<BlockUIContainer>());
            view.Markdown="正文继续";
            Assert.Empty(view.Document.Blocks.OfType<BlockUIContainer>());
            Assert.Equal("正文继续",view.PlainText);
        });
    }

    private static void RunSta(Action action)
    {
        Exception? failure=null;
        var thread=new Thread(()=>{try{action();}catch(Exception error){failure=error;}finally{Dispatcher.CurrentDispatcher.InvokeShutdown();}}){IsBackground=true};
        thread.SetApartmentState(ApartmentState.STA);thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(30)));
        if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }
}

// Emoji.Wpf caches glyph drawings in a process-wide Dictionary. Production
// windows use one UI thread; parallel STA test classes must not race that cache.
[CollectionDefinition("Emoji WPF",DisableParallelization=true)]
public sealed class EmojiWpfCollection { }
