// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Text;
namespace mewu_ai_Assistant.Services;

/// <summary>轻量级 .lnk 解析器：通过扫描二进制内容查找 UTF-16 LE 编码的
/// "X:\" 盘符路径串来推断目标路径。绕过 COM 互操作（沙箱拦截 WScript.Shell
/// 且 STAThread 限制难以满足）。当 .lnk 内含 LinkInfo/IDList 的 LocalBasePath
/// 字段时优先取该字段；否则回退到 .lnk 内嵌的 NAME String（用户为快捷方式
/// 输入的"目标"备注或 WorkingDir）。匹配成功后清掉尾部可能的脚本扩展名
///（.bat/.cmd/.lnk/.exe 等），得到安装根目录。</summary>
internal static class ShellLinkResolver
{
    internal static string? ResolveTarget(string lnkPath)
    {
        if(string.IsNullOrWhiteSpace(lnkPath)||!File.Exists(lnkPath))return null;
        byte[] data;
        try{data=File.ReadAllBytes(lnkPath);}
        catch{return null;}
        return Parse(data);
    }

    private static string? Parse(byte[] data)
    {
        if(data.Length<76)return null;
        // HeaderSize: must be 0x4C
        uint headerSize=ReadU32(data,0);
        if(headerSize!=0x4Cu)return null;
        // LinkFlags at offset 0x14
        uint linkFlags=ReadU32(data,0x14);

        // 1. Try LinkInfo.LocalBasePath / LocalBasePathUnicode.
        //    IDListSize is at offset 0x4C as DWORD; sometimes IDList bytes are
        //    truncated/corrupted which makes IDListSize unreliable, so probe
        //    the candidate LinkInfo offsets and validate headerSize+version.
        if((linkFlags&0x2)!=0u)
        {
            int idListSize=(int)ReadU32(data,0x4C);
            int linkInfoOffset=0x4C+4+(idListSize>0 && idListSize<data.Length?idListSize:0);
            if(linkInfoOffset+28<=data.Length)
            {
                uint linkInfoSize=ReadU32(data,linkInfoOffset);
                if(linkInfoSize>=28&&linkInfoSize<2000)
                {
                    uint headerSize2=ReadU32(data,linkInfoOffset+4);
                    uint ver=ReadU32(data,linkInfoOffset+8);
                    if(headerSize2>=28&&ver<=1)
                    {
                        uint flags=ReadU32(data,linkInfoOffset+0xC);
                        uint localBasePathOffset=ReadU32(data,linkInfoOffset+0x10);
                        uint localBasePathOffsetUnicode=ver>=1u?ReadU32(data,linkInfoOffset+0x14):0u;
                        if((flags&0x2)==0x2&&localBasePathOffsetUnicode>0&&localBasePathOffsetUnicode<linkInfoSize)
                        {
                            string? unicode=ReadUtf16Z(data,linkInfoOffset+(int)localBasePathOffsetUnicode);
                            if(!string.IsNullOrWhiteSpace(unicode))return StripFileName(unicode);
                        }
                        if(localBasePathOffset>0&&localBasePathOffset<linkInfoSize)
                        {
                            string? ascii=ReadAsciiZ(data,linkInfoOffset+(int)localBasePathOffset);
                            if(!string.IsNullOrWhiteSpace(ascii))return StripFileName(ascii);
                        }
                    }
                }
            }
        }

        // 2. Fallback: scan the entire file for UTF-16 LE path strings. The .lnk
        //    format embeds Name/WorkingDir/Arguments/IconLocation as Unicode strings
        //    even when LinkInfo is missing.
        var paths=ScanUtf16Paths(data);
        // Prefer longest path that contains "scrapling" or "scrap" hint
        string? best=null;
        foreach(var p in paths)
        {
            if(p.Length<4)continue;
            if(!IsPlausibleWindowsPath(p))continue;
            if(best is null||p.Length>best.Length)
            {
                var lowered=p.ToLowerInvariant();
                if(lowered.Contains("scrapling")||lowered.Contains("scrap")||lowered.Contains("python")||lowered.Contains("venv"))
                    best=p;
            }
        }
        if(best is not null)return StripFileName(best);
        // Otherwise first plausible path
        if(paths.Count>0)return StripFileName(paths[0]);
        return null;
    }

    private static string? ReadUtf16Z(byte[] data,int start)
    {
        if(start<0||start>=data.Length)return null;
        var sb=new StringBuilder();
        int i=start;
        while(i+1<data.Length)
        {
            char ch=(char)(data[i]|(data[i+1]<<8));
            if(ch=='\0')break;
            if(ch<' '&&ch!='\t')break;
            sb.Append(ch);
            i+=2;
        }
        return sb.Length==0?null:sb.ToString();
    }

    private static string? ReadAsciiZ(byte[] data,int start)
    {
        if(start<0||start>=data.Length)return null;
        int end=System.Array.IndexOf<byte>(data,0,start);
        if(end<0)end=data.Length;
        int len=end-start;
        if(len<=0)return null;
        return System.Text.Encoding.ASCII.GetString(data,start,len);
    }

    private static System.Collections.Generic.List<string> ScanUtf16Paths(byte[] data)
    {
        var result=new System.Collections.Generic.List<string>();
        for(int i=0;i<data.Length-4;i++)
        {
            if(data[i]>=0x41&&data[i]<=0x5A&&data[i+1]==0
                &&data[i+2]==0x3A&&data[i+3]==0)
            {
                int end=i;
                while(end+1<data.Length)
                {
                    char ch=(char)(data[end]|(data[end+1]<<8));
                    if(ch=='\0')break;
                    end+=2;
                }
                if(end-i>=8)
                {
                    try
                    {
                        string s=System.Text.Encoding.Unicode.GetString(data,i,end-i);
                        if(s.Length>=4&&(s.Contains('\\')||s.Contains('/')))
                            result.Add(s);
                    }catch{}
                }
            }
        }
        return result;
    }

    private static bool IsPlausibleWindowsPath(string p)
    {
        if(string.IsNullOrWhiteSpace(p))return false;
        if(p.Length<4)return false;
        if(p[1]!=':'||p[2]!='\\'&&p[2]!='/')return false;
        foreach(var c in p)
        {
            if(c<' ')return false;
            if(c=='\n'||c=='\r'||c=='\t')return false;
        }
        // 不要包含 control char
        return true;
    }

    /// <summary>把 "X:\path\to\file.exe" 化为 "X:\path\to"——.lnk 指向脚本时去掉
    /// 脚本文件名（启动脚本通常就在 scrapling 安装根目录）。</summary>
    private static string? StripFileName(string? target)
    {
        if(string.IsNullOrWhiteSpace(target))return null;
        int lastSlash=target.LastIndexOfAny(new[]{'\\','/'});
        if(lastSlash<=2)return target;
        // 如果最后一段看起来像脚本/可执行文件，则截掉
        var last=target[(lastSlash+1)..];
        if(last.Contains('.')||last.EndsWith(".bat",System.StringComparison.OrdinalIgnoreCase)
            ||last.EndsWith(".cmd",System.StringComparison.OrdinalIgnoreCase)
            ||last.EndsWith(".exe",System.StringComparison.OrdinalIgnoreCase)
            ||last.EndsWith(".lnk",System.StringComparison.OrdinalIgnoreCase)
            ||last.EndsWith(".py",System.StringComparison.OrdinalIgnoreCase))
        {
            return target[..lastSlash];
        }
        return target;
    }

    private static uint ReadU32(byte[] data,int offset)
        =>(uint)(data[offset]|(data[offset+1]<<8)|(data[offset+2]<<16)|(data[offset+3]<<24));
}