// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using TextBox=System.Windows.Controls.TextBox;

internal static class PromptRevealFocusReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;

    internal static void Run(Application app,CaptureOverlayWindow overlay)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        overlay.Loaded+=(_,_)=>
        {
            app.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,new Action(async()=>
            {
                var checks=new List<string>();string? failure=null;
                try
                {
                    var root=(Canvas)overlay.FindName("Root");
                    var prompt=(TextBox)overlay.FindName("QuickPrompt");
                    var host=(FrameworkElement)overlay.FindName("PromptBarHost");
                    var answer=(MarkdownAnswerView)overlay.FindName("AnswerText");
                    var answerHeader=(FrameworkElement)overlay.FindName("AnswerHeader");
                    var answerPanel=(FrameworkElement)overlay.FindName("AnswerScroll");
                    var answerSeparator=(FrameworkElement)overlay.FindName("AnswerDivider");
                    const string Draft="保留已有草稿。";
                    Check("first-reveal-focuses-input-and-keeps-answer-collapsed",prompt.IsKeyboardFocused&&
                        answerPanel.Visibility==Visibility.Collapsed&&answerHeader.Visibility==Visibility.Collapsed&&answerSeparator.Visibility==Visibility.Collapsed);
                    prompt.Text=Draft;
                    await ReplayReveal("hide-and-reveal");
                    Check("existing-draft-and-caret-preserved",prompt.Text==Draft&&prompt.CaretIndex==Draft.Length);

                    prompt.CaretIndex=3;
                    HideShow(false);
                    await Yield();
                    Check("already-visible-refresh-keeps-caret",prompt.CaretIndex==3);
                    Invoke("ShowAnswer");
                    Invoke("RefreshAnswer","回答内容，可选择和复制。");
                    await Yield();
                    Invoke("PositionPromptBar");overlay.UpdateLayout();
                    answer.SelectAll();answer.Focus();
                    var selection=answer.SelectedPlainText;
                    Check("answer-selection-ready",answer.IsKeyboardFocused&&!answer.Selection.IsEmpty);
                    Invoke("ShowAnswer");
                    Invoke("PositionPromptBar");
                    HideShow(false);
                    await Yield();
                    Check("streaming-and-layout-do-not-steal-answer-selection",answer.IsKeyboardFocused&&answer.SelectedPlainText==selection);

                    host.Visibility=Visibility.Collapsed;
                    await Yield();
                    Check("hidden-host-releases-input-focus",!prompt.IsKeyboardFocusWithin);
                    root.Focus();
                    host.Visibility=Visibility.Visible;
                    await Yield();
                    Check("restoring-host-visibility-focuses-input",prompt.IsKeyboardFocused);

                    HideShow(true);
                    root.Focus();
                    HideShow(false);
                    HideShow(true);
                    await Yield();
                    Check("late-reveal-cannot-focus-hidden-input",!prompt.IsKeyboardFocused&&(bool)Get("_promptBarHidden")!);

                    await ReplayReveal("repeated-reveal-always-focuses-input");
                    Invoke("SetPromptBarHidden",true,false);
                    root.Focus();
                    Set("_conversationAiAvailable",false);
                    HideShow(false);
                    await Yield();
                    Check("offline-capture-does-not-focus-hidden-composer",!prompt.IsKeyboardFocused&&host.Visibility==Visibility.Collapsed);
                    Set("_conversationAiAvailable",true);
                    host.Visibility=Visibility.Visible;
                    HideShow(false);
                    await Yield();
                    Check("composer-availability-restored-focuses-input",prompt.IsKeyboardFocused);

                    HideShow(true);
                    HideShow(false);
                    overlay.Close();
                    await Yield();
                    Check("closing-discards-queued-focus",(bool)Get("_closed")!&&!prompt.IsKeyboardFocused);

                    async Task ReplayReveal(string name)
                    {
                        HideShow(true);
                        root.Focus();
                        HideShow(false);
                        await Yield();
                        Check(name,prompt.IsKeyboardFocused&&!(bool)Get("_promptBarHidden")!);
                    }
                    void HideShow(bool hidden)=>Invoke("SetPromptBarHidden",hidden,false);
                }
                catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
                finally
                {
                    Directory.CreateDirectory(".codex-build");
                    File.WriteAllText(".codex-build/prompt-reveal-focus-result.json",JsonSerializer.Serialize(new{checks,failure}));
                    overlay.Close();app.Shutdown(Environment.ExitCode);
                }
                object? Get(string name)=>overlay.GetType().GetField(name,Private)!.GetValue(overlay);
                void Set(string name,object? value)=>overlay.GetType().GetField(name,Private)!.SetValue(overlay,value);
                object? Invoke(string name,params object?[] values)=>overlay.GetType().GetMethod(name,Private)!.Invoke(overlay,values);
                void Check(string name,bool condition){if(!condition)throw new InvalidOperationException(name);checks.Add(name);}
            }));
        };
    }

    private static async Task Yield()=>await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
}
