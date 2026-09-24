// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.RegularExpressions;
namespace mewu_ai_Assistant.Services;

/// <summary>
/// OCR/译文文本中的电话号码与长数字串识别。
/// 检测到号码时，文本右键菜单会追加 拨号（tel:）/ 短信（sms:）/ 复制 快捷操作；
/// 长数字串（订单号等，非电话形态）只提供复制。纯本地识别，不发起任何网络请求。
/// </summary>
internal static partial class PhoneNumberService
{
    /// <summary>可拨号形态：中国大陆手机号。</summary>
    [GeneratedRegex(@"(?<![\d-])(?:\+?86[ -]?)?1[3-9](?:[ -]?\d){9}(?![\d-])")]
    private static partial Regex MobileRegex();
    /// <summary>可拨号形态：区号座机（010-12345678 / 0755 1234567）。</summary>
    [GeneratedRegex(@"(?<![\d-])0\d{2,3}[ -]?\d{7,8}(?![\d-])")]
    private static partial Regex LandlineRegex();
    /// <summary>可拨号形态：400/800 热线（400-123-4567）。</summary>
    [GeneratedRegex(@"(?<![\d-])[48]00[ -]?\d{3,4}[ -]?\d{3,4}(?![\d-])")]
    private static partial Regex HotlineRegex();
    /// <summary>长数字串（含字母数字组合的订单号/编号），仅提供复制。</summary>
    [GeneratedRegex(@"(?<![\dA-Za-z])[A-Za-z]?\d{7,20}[A-Za-z]?(?![\dA-Za-z])")]
    private static partial Regex LongDigitRegex();

    internal sealed record Candidate(string Display,bool Dialable);

    /// <summary>整段选中文本是否是一个号码（用于右键菜单即时判定）。</summary>
    internal static Candidate? Classify(string text)
    {
        if(string.IsNullOrWhiteSpace(text))return null;
        var trimmed=text.Trim();
        if(trimmed.Length is <7 or >24)return null;
        if(MobileRegex().IsMatch(trimmed)||LandlineRegex().IsMatch(trimmed)||HotlineRegex().IsMatch(trimmed))
            return new Candidate(NormalizeDisplay(trimmed),true);
        if(LongDigitRegex().IsMatch(trimmed)&&trimmed.All(char.IsLetterOrDigit))
            return new Candidate(trimmed,false);
        return null;
    }

    /// <summary>从整段文本中提取号码候选（去重、保持出现顺序、最多 4 个）。</summary>
    internal static IReadOnlyList<Candidate> Find(string text)
    {
        if(string.IsNullOrWhiteSpace(text))return[];
        var results=new List<Candidate>();
        void Add(MatchCollection matches,bool dialable)
        {
            foreach(Match match in matches)
            {
                var value=NormalizeDisplay(match.Value);
                if(results.All(entry=>entry.Display!=value))
                    results.Add(new Candidate(value,dialable));
                if(results.Count>=4)return;
            }
        }
        Add(MobileRegex().Matches(text),true);
        Add(LandlineRegex().Matches(text),true);
        Add(HotlineRegex().Matches(text),true);
        if(results.Count<4)Add(LongDigitRegex().Matches(text),false);
        return results.Take(4).ToList();
    }

    /// <summary>经系统默认应用向号码发起呼叫（tel: URI）。</summary>
    internal static void Dial(string number)=>Launch($"tel:{Normalize(number)}");
    /// <summary>经系统默认应用向号码发短信（sms: URI）。</summary>
    internal static void SendSms(string number)=>Launch($"sms:{Normalize(number)}");

    private static void Launch(string uri)
    {
        try{System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(uri){UseShellExecute=true});}
        catch(Exception ex){new PrivacyLogger().Info("PhoneNumberLaunch",ex.GetType().Name);}
    }

    /// <summary>拨号前去掉空格与连字符，保留数字与起始 +。</summary>
    private static string Normalize(string number)
    {
        var trimmed=number.Trim().Replace(" ","").Replace("-","");
        return trimmed.StartsWith('+')?trimmed:trimmed;
    }

    private static string NormalizeDisplay(string number)
        => number.Trim().Replace(" ",string.Empty).Replace("-",string.Empty);
}
