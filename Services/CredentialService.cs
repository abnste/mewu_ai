using System.Runtime.InteropServices;
using System.Security.Cryptography;
namespace mewu_ai_Assistant.Services;
public sealed class CredentialService
{
    private const uint CryptProtectUiForbidden=0x1;
    private readonly string _directory;
    public CredentialService(string? directory=null){_directory=directory??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","Credentials");Directory.CreateDirectory(_directory);}
    public void Save(string id,string secret){var path=PathFor(id);var temp=path+"."+Guid.NewGuid().ToString("N")+".tmp";var plain=System.Text.Encoding.UTF8.GetBytes(secret);byte[]? encrypted=null;try{encrypted=Protect(plain);File.WriteAllBytes(temp,encrypted);File.Move(temp,path,true);}finally{CryptographicOperations.ZeroMemory(plain);if(encrypted is not null)CryptographicOperations.ZeroMemory(encrypted);try{if(File.Exists(temp))File.Delete(temp);}catch{}}}
    public string? Read(string id){byte[]? encrypted=null;byte[]? plain=null;try{var p=PathFor(id);if(!File.Exists(p))return null;encrypted=File.ReadAllBytes(p);plain=Unprotect(encrypted);return System.Text.Encoding.UTF8.GetString(plain);}catch{return null;}finally{if(plain is not null)CryptographicOperations.ZeroMemory(plain);if(encrypted is not null)CryptographicOperations.ZeroMemory(encrypted);}}
    public void Delete(string id){if(string.IsNullOrWhiteSpace(id))return;var path=PathFor(id);if(File.Exists(path))File.Delete(path);}
    private string PathFor(string id){if(string.IsNullOrWhiteSpace(id)||id.Length>128||id.Any(c=>!char.IsLetterOrDigit(c)&&c is not '-' and not '_'))throw new ArgumentException("凭据标识无效",nameof(id));return Path.Combine(_directory,id+".bin");}
    private static byte[] Protect(byte[] data)=>Crypt(data,true);private static byte[] Unprotect(byte[] data)=>Crypt(data,false);
    private static byte[] Crypt(byte[] data,bool protect)
    {
        var input=new DataBlob();var output=new DataBlob();try{input.Data=Marshal.AllocHGlobal(data.Length);input.Length=data.Length;Marshal.Copy(data,0,input.Data,data.Length);var ok=protect?CryptProtectData(ref input,null,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,CryptProtectUiForbidden,ref output):CryptUnprotectData(ref input,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,CryptProtectUiForbidden,ref output);if(!ok)throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());var result=new byte[output.Length];try{Marshal.Copy(output.Data,result,0,result.Length);return result;}catch{CryptographicOperations.ZeroMemory(result);throw;}}finally{ZeroMemory(input.Data,input.Length);ZeroMemory(output.Data,output.Length);if(input.Data!=IntPtr.Zero)Marshal.FreeHGlobal(input.Data);if(output.Data!=IntPtr.Zero)LocalFree(output.Data);}
    }
    private static void ZeroMemory(IntPtr memory,int length)
    {
        if(memory==IntPtr.Zero||length<=0)return;
        var zeros=new byte[Math.Min(length,4096)];
        for(var offset=0;offset<length;offset+=zeros.Length)Marshal.Copy(zeros,0,IntPtr.Add(memory,offset),Math.Min(zeros.Length,length-offset));
    }
    [StructLayout(LayoutKind.Sequential)]private struct DataBlob{public int Length;public IntPtr Data;}
    [DllImport("crypt32.dll",SetLastError=true,CharSet=CharSet.Unicode)]private static extern bool CryptProtectData(ref DataBlob data,string? description,IntPtr entropy,IntPtr reserved,IntPtr prompt,uint flags,ref DataBlob output);
    [DllImport("crypt32.dll",SetLastError=true,CharSet=CharSet.Unicode)]private static extern bool CryptUnprotectData(ref DataBlob data,IntPtr description,IntPtr entropy,IntPtr reserved,IntPtr prompt,uint flags,ref DataBlob output);
    [DllImport("kernel32.dll")]private static extern IntPtr LocalFree(IntPtr memory);
}
