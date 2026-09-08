// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Point=System.Windows.Point;

internal static class CaptureHoverReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic|BindingFlags.DeclaredOnly;

    internal static void Run(Application app,CaptureOverlayWindow overlay)
    {
        // Replay coordinates directly through the production pointer handler;
        // live desktop mouse movement must not race the regression scenarios.
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;overlay.IsHitTestVisible=false;overlay.Show();
        app.Dispatcher.BeginInvoke(DispatcherPriority.Normal,new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;
            try
            {
                var root=(Canvas)overlay.FindName("Root");
                var toolbar=(FrameworkElement)overlay.FindName("Toolbar");
                var prompt=(FrameworkElement)overlay.FindName("PromptBarHost");
                var a=Add(new Rect(100,150,360,180));var b=Add(new Rect(650,150,180,180));
                Invoke("Select",0);Invoke("PositionPromptBar");Invoke("ShowToolbar");
                await Move(new Point(200,220));
                Require(toolbar.Visibility==Visibility.Visible&&Hidden(),$"Selection hover did not show tools and hide composer: toolbar={toolbar.Visibility}, hidden={Hidden()}, pointer={Mouse.GetPosition(root)}, active={Get("_activeIndex")}, prompt={Invoke("GetPromptInteractionBounds")}, promptHover={Invoke("IsInteractingWithPrompt",Mouse.GetPosition(root))}");
                checks.Add("selection-hover-shows-tools");

                await Move(new Point(20,450));
                Require(toolbar.Visibility!=Visibility.Visible&&!Hidden(),"Leaving all regions left toolbar visible or composer hidden");
                checks.Add("leave-selection-hides-tools-and-restores-composer");

                await Move(new Point(200,220));overlay.UpdateLayout();
                var bounds=new Rect(Canvas.GetLeft(toolbar),Canvas.GetTop(toolbar),toolbar.ActualWidth,toolbar.ActualHeight);
                b.GetType().GetField("Bounds")!.SetValue(b,bounds);Invoke("UpdateSelection",b);
                await Move(new Point(bounds.Left+60,bounds.Top+bounds.Height/2));
                Require((int)Get("_activeIndex")==0&&toolbar.Visibility==Visibility.Visible,$"Toolbar overlap switched owner or hid tools: active={Get("_activeIndex")}, visibility={toolbar.Visibility}, bounds={bounds}, pointer={Mouse.GetPosition(root)}, captured={Mouse.Captured}");
                checks.Add("toolbar-over-another-region-retains-owner");

                using(var pending=new CancellationTokenSource())
                {
                    Set("_overlayRequest",pending);
                    await Move(new Point(20,450));
                    Require(!Hidden()&&toolbar.Visibility!=Visibility.Visible,"Pending translation locked composer hidden");
                    await Move(new Point(200,220));
                    Require(Hidden(),"Pending translation no longer respects selection hover");
                    await Move(new Point(20,450));Set("_overlayRequest",null);
                }
                checks.Add("pending-translation-does-not-lock-visibility");

                var promptBounds=(Rect)Invoke("GetPromptInteractionBounds")!;
                var promptPoint=new Point(promptBounds.Left+promptBounds.Width/2,promptBounds.Top+promptBounds.Height/2);
                a.GetType().GetField("Bounds")!.SetValue(a,promptBounds);Invoke("UpdateSelection",a);
                await Move(promptPoint);
                Invoke("RefreshToolbar",promptPoint);Invoke("PositionPromptBar");
                Require(!Hidden()&&toolbar.Visibility!=Visibility.Visible,"Prompt hover or late layout revived selection tools");
                for(var i=0;i<5;i++)await Move(promptPoint);
                Require(!Hidden()&&toolbar.Visibility!=Visibility.Visible,"Stationary prompt hover oscillated");
                checks.Add("prompt-hover-wins-and-late-layout-cannot-revive-tools");

                var monitor=(Rect)Invoke("PromptMonitorBounds")!;
                var bottomSelection=new Rect(promptBounds.Left,monitor.Bottom-200,promptBounds.Width,200);
                a.GetType().GetField("Bounds")!.SetValue(a,bottomSelection);Invoke("UpdateSelection",a);
                Invoke("SetPromptBarHidden",true,true);
                await Move(new Point(promptPoint.X,monitor.Bottom-5));
                Require(!Hidden()&&toolbar.Visibility!=Visibility.Visible,"Bottom selection blocked composer recovery");
                await Move(new Point(promptPoint.X,monitor.Bottom-5));
                Require(!Hidden(),"Screen edge recovery oscillated");
                checks.Add("bottom-edge-recovers-composer-over-partial-selection");
                overlay.IsHitTestVisible=true;
                Require(prompt.IsHitTestVisible,"Restored composer cannot receive input");

                object Add(Rect rectangle)
                {
                    var item=Invoke("CreateSelection",false)!;item.GetType().GetField("Bounds")!.SetValue(item,rectangle);
                    ((IList)Get("_selections")).Add(item);Invoke("UpdateSelection",item);return item;
                }
                Task Move(Point point)
                {
                    Invoke("UpdatePointerInteraction",point);overlay.UpdateLayout();
                    return Task.CompletedTask;
                }
                bool Hidden()=>(bool)Get("_promptBarHidden");
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                overlay.Close();Directory.CreateDirectory(".codex-build");
                File.WriteAllText(".codex-build/capture-hover-result.json",JsonSerializer.Serialize(new{checks,failure}));
                app.Shutdown(Environment.ExitCode);
            }
        }));
        object Get(string field)=>overlay.GetType().GetField(field,Private)!.GetValue(overlay)!;
        void Set(string field,object? value)=>overlay.GetType().GetField(field,Private)!.SetValue(overlay,value);
        object? Invoke(string method,params object[] args)=>overlay.GetType().GetMethod(method,Private)!.Invoke(overlay,args);
    }
    private static void Require(bool condition,string message){if(!condition)throw new InvalidOperationException(message);}
}
