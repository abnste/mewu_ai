using System.Windows.Input;
using System.Windows.Interop;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
public sealed class GlobalHotkeyService : IDisposable
{
    private const int Id=0x4D57; private readonly HwndSource _source;
    public event Action? Pressed;
    public GlobalHotkeyService()
    {
        _source=new HwndSource(new HwndSourceParameters("MewuAI.Hotkey") { Width=0,Height=0,WindowStyle=unchecked((int)0x80000000) });
        _source.AddHook(WndProc);
    }
    public bool Register(HotkeySetting hotkey)
    {
        NativeMethods.UnregisterHotKey(_source.Handle,Id);
        var modifiers=(uint)hotkey.Modifiers | 0x4000u;
        return NativeMethods.RegisterHotKey(_source.Handle,Id,modifiers,(uint)KeyInterop.VirtualKeyFromKey(hotkey.Key));
    }
    private IntPtr WndProc(IntPtr hwnd,int msg,IntPtr wParam,IntPtr lParam,ref bool handled) { if(msg==NativeMethods.WmHotkey&&wParam.ToInt32()==Id){handled=true;Pressed?.Invoke();} return IntPtr.Zero; }
    public void Dispose() { NativeMethods.UnregisterHotKey(_source.Handle,Id); _source.RemoveHook(WndProc); _source.Dispose(); }
}
