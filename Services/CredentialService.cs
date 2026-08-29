using System.Runtime.InteropServices;
namespace mewu_ai_Assistant.Services;
public sealed class CredentialService
{
    private readonly string _directory;
    public CredentialService(string? directory=null){_directory=directory??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Credentials");Directory.CreateDirectory(_directory);}
    public void Save(string id,string secret){var plain=System.Text.Encoding.UTF8.GetBytes(secret);try{var encrypted=Protect(plain);File.WriteAllBytes(PathFor(id),encrypted);}finally{Array.Clear(plain);}}
    public string? Read(string id){try{var p=PathFor(id);if(!File.Exists(p))return null;var plain=Unprotect(File.ReadAllBytes(p));try{return System.Text.Encoding.UTF8.GetString(plain);}finally{Array.Clear(plain);}}catch{return null;}}
    public void Delete(string id){if(string.IsNullOrWhiteSpace(id))return;var path=PathFor(id);if(File.Exists(path))File.Delete(path);}
    private string PathFor(string id){if(string.IsNullOrWhiteSpace(id)||id.Length>128||id.Any(c=>!char.IsLetterOrDigit(c)&&c is not '-' and not '_'))throw new ArgumentException("凭据标识无效",nameof(id));return Path.Combine(_directory,id+".bin");}
    private static byte[] Protect(byte[] data)=>Crypt(data,true);private static byte[] Unprotect(byte[] data)=>Crypt(data,false);
    private static byte[] Crypt(byte[] data,bool protect)
    {
        var input=new DataBlob();var output=new DataBlob();try{input.Data=Marshal.AllocHGlobal(data.Length);input.Length=data.Length;Marshal.Copy(data,0,input.Data,data.Length);var ok=protect?CryptProtectData(ref input,null,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,0,ref output):CryptUnprotectData(ref input,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,0,ref output);if(!ok)throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());var result=new byte[output.Length];Marshal.Copy(output.Data,result,0,result.Length);return result;}finally{if(input.Data!=IntPtr.Zero)Marshal.FreeHGlobal(input.Data);if(output.Data!=IntPtr.Zero)LocalFree(output.Data);}
    }
    [StructLayout(LayoutKind.Sequential)]private struct DataBlob{public int Length;public IntPtr Data;}
    [DllImport("crypt32.dll",SetLastError=true,CharSet=CharSet.Unicode)]private static extern bool CryptProtectData(ref DataBlob data,string? description,IntPtr entropy,IntPtr reserved,IntPtr prompt,uint flags,ref DataBlob output);
    [DllImport("crypt32.dll",SetLastError=true,CharSet=CharSet.Unicode)]private static extern bool CryptUnprotectData(ref DataBlob data,IntPtr description,IntPtr entropy,IntPtr reserved,IntPtr prompt,uint flags,ref DataBlob output);
    [DllImport("kernel32.dll")]private static extern IntPtr LocalFree(IntPtr memory);
}
