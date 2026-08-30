using System.Windows.Input;
using System.Windows.Interop;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.Services;
public sealed class GlobalHotkeyService : IDisposable
{
    private const int PrimaryId=0x4D57,SecondaryId=0x4D58; private readonly HwndSource _source;private HotkeySetting? _current;private int _currentId;
    public event Action? Pressed;
    public GlobalHotkeyService()
    {
        _source=new HwndSource(new HwndSourceParameters("MewuAI.Hotkey") { Width=0,Height=0,WindowStyle=unchecked((int)0x80000000) });
        _source.AddHook(WndProc);
    }
    public bool Register(HotkeySetting hotkey)
    {
        ArgumentNullException.ThrowIfNull(hotkey);
        if(_current is not null&&_current.Key==hotkey.Key&&_current.Modifiers==hotkey.Modifiers)return true;
        var candidateId=_current is null?PrimaryId:_currentId==PrimaryId?SecondaryId:PrimaryId;
        var modifiers=(uint)hotkey.Modifiers | 0x4000u;
        if(!NativeMethods.RegisterHotKey(_source.Handle,candidateId,modifiers,(uint)KeyInterop.VirtualKeyFromKey(hotkey.Key)))return false;
        if(_current is not null)NativeMethods.UnregisterHotKey(_source.Handle,_currentId);
        _currentId=candidateId;_current=new HotkeySetting{Key=hotkey.Key,Modifiers=hotkey.Modifiers};return true;
    }
    private IntPtr WndProc(IntPtr hwnd,int msg,IntPtr wParam,IntPtr lParam,ref bool handled) { if(msg==NativeMethods.WmHotkey&&wParam.ToInt32()==_currentId){handled=true;Pressed?.Invoke();} return IntPtr.Zero; }
    public void Dispose() { if(_current is not null)NativeMethods.UnregisterHotKey(_source.Handle,_currentId); _source.RemoveHook(WndProc); _source.Dispose(); }
}
