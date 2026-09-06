// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Reflection;
using System.Runtime.ExceptionServices;
using System.Windows.Input;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class HotkeyTests
{
    [Fact]
    public void DisabledShortcutSurvivesSavingAndRestartAndCanBeRestored()
    {
        var directory=Path.Combine(Path.GetTempPath(),"MewuAI.Tests",Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            var service=new SettingsService(Path.Combine(directory,"settings.json"));
            var settings=new AppSettings
            {
                CaptureHotkey=new(){Key=Key.None,Modifiers=ModifierKeys.Alt},
                DefaultProviderId="test",
                Providers=[new(){Id="test",CredentialId="test-credential"}]
            };
            service.Save(settings);
            var loaded=service.Load();
            Assert.Equal(Key.None,loaded.CaptureHotkey.Key);
            Assert.Equal(ModifierKeys.None,loaded.CaptureHotkey.Modifiers);
            service.Save(loaded);
            Assert.Equal(Key.None,service.Load().CaptureHotkey.Key);
            loaded.CaptureHotkey=new();service.Save(loaded);
            Assert.Equal(Key.S,service.Load().CaptureHotkey.Key);
            Assert.Equal(ModifierKeys.Shift|ModifierKeys.Alt,service.Load().CaptureHotkey.Modifiers);
        }
        finally{Directory.Delete(directory,true);}
    }

    [Fact]
    public void ClearingReleasesSystemShortcutAndIgnoresQueuedMessages()
    {
        Exception? failure=null;
        var thread=new Thread(()=>
        {
            try
            {
                using var service=new GlobalHotkeyService();
                using var other=new GlobalHotkeyService();
                var disabled=new HotkeySetting{Key=Key.None,Modifiers=ModifierKeys.None};
                Assert.True(service.Register(disabled));
                HotkeySetting? selected=null;
                foreach(var key in new[]{Key.Z,Key.Y,Key.X,Key.W,Key.V})
                {
                    var candidate=new HotkeySetting{Key=key,Modifiers=ModifierKeys.Control|ModifierKeys.Shift|ModifierKeys.Alt};
                    if(service.Register(candidate)){selected=candidate;break;}
                }
                Assert.NotNull(selected);
                Assert.False(other.Register(selected));
                var flags=BindingFlags.Instance|BindingFlags.NonPublic;
                var oldId=(int)typeof(GlobalHotkeyService).GetField("_currentId",flags)!.GetValue(service)!;
                var pressed=0;service.Pressed+=()=>pressed++;
                Assert.True(service.Register(disabled));
                Assert.True(service.Register(disabled));
                typeof(GlobalHotkeyService).GetMethod("WndProc",flags)!.Invoke(service,[IntPtr.Zero,0x0312,new IntPtr(oldId),IntPtr.Zero,false]);
                Assert.Equal(0,pressed);
                Assert.True(other.Register(selected));
                Assert.True(other.Register(disabled));
                Assert.True(service.Register(selected));
            }
            catch(Exception ex){failure=ex;}
        }){IsBackground=true};
        thread.SetApartmentState(ApartmentState.STA);thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(20)),"Hotkey lifecycle test timed out");
        if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
