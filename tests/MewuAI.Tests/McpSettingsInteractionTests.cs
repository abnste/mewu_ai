// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Reflection;
using System.Runtime.ExceptionServices;
using System.Windows.Controls;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Xunit;

namespace MewuAI.Tests;

public sealed class McpSettingsInteractionTests
{
    [Fact]
    public void ImaSnapshotsControlsOnUiThreadAndKeepsExplicitTarget()
        =>RunSta(async()=>
        {
            var uiThread=Environment.CurrentManagedThreadId;
            var called=false;
            var page=ImaPage(new(){ImaClientId="client-a",ImaKnowledgeBaseId="chosen",ImaKnowledgeBaseName="Chosen"},(settings,_)=>
            {
                called=true;Assert.NotEqual(uiThread,Environment.CurrentManagedThreadId);
                Assert.Equal("client-a",settings.ImaClientId);Assert.Equal("chosen",settings.ImaKnowledgeBaseId);
                return Task.FromResult<IReadOnlyList<ImaVaultService.ImaKnowledgeBase>>([new("first","First"),new("chosen","Chosen")]);
            });
            await page.TestConnectionAsync();
            Assert.True(called);Assert.Equal("chosen",page.KnowledgeBaseId);
            Assert.True(Field<Button>(page,"_test").IsEnabled);
        });

    [Fact]
    public void ImaMissingSavedTargetDoesNotSilentlySelectAnotherLibrary()
        =>RunSta(async()=>
        {
            var page=ImaPage(new(){ImaClientId="client",ImaKnowledgeBaseId="missing",ImaKnowledgeBaseName="Saved"},(_,_)=>
                Task.FromResult<IReadOnlyList<ImaVaultService.ImaKnowledgeBase>>([new("different","Different")]));
            await page.TestConnectionAsync();
            Assert.Equal("missing",page.KnowledgeBaseId);Assert.Equal("Saved",page.KnowledgeBaseName);
            Assert.Equal(3,Field<ComboBox>(page,"_knowledgeBase").Items.Count);
        });

    [Fact]
    public void ImaDefaultSelectionRemainsDefaultAfterLoading()
        =>RunSta(async()=>
        {
            var page=ImaPage(new(){ImaClientId="client"},(_,_)=>Task.FromResult<IReadOnlyList<ImaVaultService.ImaKnowledgeBase>>([new("first","First")]));
            await page.TestConnectionAsync();Assert.Empty(page.KnowledgeBaseId);
            Field<ComboBox>(page,"_knowledgeBase").SelectedIndex=1;Assert.Equal("first",page.KnowledgeBaseId);
            Field<ComboBox>(page,"_knowledgeBase").SelectedIndex=0;Assert.Empty(page.KnowledgeBaseId);
        });

    [Fact]
    public void ImaChangedClientRejectsOldResponse()
        =>RunSta(async()=>
        {
            var response=new TaskCompletionSource<IReadOnlyList<ImaVaultService.ImaKnowledgeBase>>(TaskCreationOptions.RunContinuationsAsynchronously);
            var page=ImaPage(new(){ImaClientId="old",ImaKnowledgeBaseId="old-library"},(_,_)=>response.Task);
            var request=page.TestConnectionAsync();
            Field<TextBox>(page,"_clientId").Text="new";
            response.SetResult([new("old-library","Old")]);await request;
            Assert.Empty(page.KnowledgeBaseId);Assert.Single(Field<ComboBox>(page,"_knowledgeBase").Items.Cast<object>());
        });

    [Fact]
    public void ImaTestingUnsavedKeyDoesNotPersistOrCallNetwork()
        =>RunSta(async()=>
        {
            var page=ImaPage(new(){ImaClientId="client"},(_,_)=>throw new InvalidOperationException("Network must not run"));
            Field<PasswordBox>(page,"_apiKey").Password="synthetic-unsaved";
            await page.TestConnectionAsync();
            Assert.Equal("synthetic-unsaved",Field<PasswordBox>(page,"_apiKey").Password);
            Assert.True(Field<Button>(page,"_test").IsEnabled);
        });

    [Fact]
    public void ImaCancelledWindowIgnoresCompletedLibraryList()
        =>RunSta(async()=>
        {
            using var cancellation=new CancellationTokenSource();
            var response=new TaskCompletionSource<IReadOnlyList<ImaVaultService.ImaKnowledgeBase>>(TaskCreationOptions.RunContinuationsAsynchronously);
            var page=ImaPage(new(){ImaClientId="client",ImaKnowledgeBaseId="saved"},(_,_)=>response.Task,cancellation.Token);
            var request=page.TestConnectionAsync();cancellation.Cancel();response.SetResult([new("new","New")]);
            await request;Assert.Equal("saved",page.KnowledgeBaseId);
        });

    [Fact]
    public void FeishuSelectingChatUpdatesReceiverTypeAndRetainsEveryReturnedChat()
        =>RunSta(async()=>
        {
            var page=new FeishuSettingsPage(new(){FeishuAppId="app",FeishuTargetType="open_id",FeishuTargetId="contact"},CancellationToken.None,
                (_,_)=>Task.FromResult<IReadOnlyList<(string ChatId,string Name)>>(Enumerable.Range(0,75).Select(i=>($"chat-{i}",$"Chat {i}")).ToArray()));
            await page.TestAsync();
            var chats=Field<ComboBox>(page,"_chats");Assert.Equal(75,chats.Items.Count);
            chats.SelectedIndex=74;Assert.Equal("chat-74",page.TargetId);Assert.Equal("chat_id",page.TargetType);
            Field<ComboBox>(page,"_targetType").SelectedIndex=1;Assert.Null(chats.SelectedItem);
        });

    [Fact]
    public void FeishuChangedAppRejectsOldChatList()
        =>RunSta(async()=>
        {
            var response=new TaskCompletionSource<IReadOnlyList<(string ChatId,string Name)>>(TaskCreationOptions.RunContinuationsAsynchronously);
            var page=new FeishuSettingsPage(new(){FeishuAppId="old"},CancellationToken.None,(_,_)=>response.Task);
            var request=page.TestAsync();Field<TextBox>(page,"_appId").Text="new";
            response.SetResult([("old-chat","Old")]);await request;
            Assert.Empty(Field<ComboBox>(page,"_chats").Items.Cast<object>());
        });

    [Theory]
    [InlineData("{}")]
    [InlineData("{\"data\":{\"data\":{\"aliases\":[]}}}")]
    [InlineData("{\"data\":{\"aliases\":[{\"email\":\" \"}]}}")]
    [InlineData("{\"data\":{\"aliases\":[{\"email\":123}]}}")]
    public void QqMissingAccountCannotPassConnectionTest(string json)
        =>Assert.Throws<InvalidOperationException>(()=>QqMailSettingsPage.ParseAccount(json));

    [Theory]
    [InlineData("{\"data\":{\"aliases\":[{\"email\":\"first@example.test\"},{\"email\":\"primary@example.test\",\"is_primary\":true}]}}")]
    [InlineData("{\"data\":{\"data\":{\"aliases\":[{\"email\":\"primary@example.test\"}]}}}")]
    public void QqRecognizesBothSupportedAccountEnvelopes(string json)
        =>Assert.Equal("primary@example.test",QqMailSettingsPage.ParseAccount(json));

    private static ImaSettingsPage ImaPage(AppSettings settings,
        Func<AppSettings,CancellationToken,Task<IReadOnlyList<ImaVaultService.ImaKnowledgeBase>>> load,CancellationToken token=default)
        =>new(settings,token,load,_=>false,_=>throw new InvalidOperationException("Unexpected credential write"),()=>throw new InvalidOperationException("Unexpected credential clear"));

    private static T Field<T>(object page,string name) where T:class
        =>(T)page.GetType().GetField(name,BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(page)!;

    private static void RunSta(Func<Task> action)
    {
        Exception? error=null;
        var thread=new Thread(()=>
        {
            var dispatcher=Dispatcher.CurrentDispatcher;
            SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(dispatcher));
            dispatcher.BeginInvoke(new Action(async()=>
            {
                try{await action();}catch(Exception ex){error=ex;}
                finally{dispatcher.BeginInvokeShutdown(DispatcherPriority.Background);}
            }));
            Dispatcher.Run();
        }){IsBackground=true};
        thread.SetApartmentState(ApartmentState.STA);thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(25)),"MCP settings test timed out");
        if(error is not null)ExceptionDispatchInfo.Capture(error).Throw();
    }
}
