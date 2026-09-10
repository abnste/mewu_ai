// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Media;
using WpfMath.Parsers;
using WpfMath.Rendering;
using XamlMath;

namespace mewu_ai_Assistant.Services;

/// <summary>Offline formula typesetting; never evaluates or rewrites stored answers.</summary>
internal static class MathFormulaRenderer
{
    private static readonly object ParserGate=new();
    private static readonly HashSet<string> Commands=new(("frac dfrac tfrac sqrt left right cdot times div pm mp le leq ge geq ne neq approx equiv infty sum prod int lim sin cos tan log ln alpha beta gamma delta theta pi sigma omega Delta Sigma Omega mathrm mathbf mathit text overline underline vec hat bar begin end quad qquad displaystyle substack cases aligned matrix pmatrix bmatrix cdots ldots vert Vert lvert rvert langle rangle").Split(' '),StringComparer.Ordinal);
    internal static DrawingImage? Create(string source,double size,Brush foreground,bool allowPlain=true,bool halo=false)
    {
        if(source.Length is 0 or >2048)return null;
        var value=source.Trim();
        if(value.StartsWith("$$",StringComparison.Ordinal)&&value.EndsWith("$$",StringComparison.Ordinal)&&value.Length>4)value=value[2..^2];
        else if(value.StartsWith('$')&&value.EndsWith('$')&&value.Length>2)value=value[1..^1];
        else if((value.StartsWith(@"\(",StringComparison.Ordinal)&&value.EndsWith(@"\)",StringComparison.Ordinal))||(value.StartsWith(@"\[",StringComparison.Ordinal)&&value.EndsWith(@"\]",StringComparison.Ordinal)))value=value[2..^2];
        else if(!value.Contains('\\'))
        {
            if(!allowPlain||!PlainMathNotation.TryConvert(value,out value))return null;
        }
        if(!IsBoundedFormula(value))return null;
        value=value.Replace(@"\begin{aligned}",@"\begin{align}",StringComparison.Ordinal).Replace(@"\end{aligned}",@"\end{align}",StringComparison.Ordinal)
            .Replace(@"\begin{align*}",@"\begin{align}",StringComparison.Ordinal).Replace(@"\end{align*}",@"\end{align}",StringComparison.Ordinal);
        try
        {
            Geometry geometry;
            lock(ParserGate)
            {
                var formula=WpfTeXFormulaParser.Instance.Parse(value);
                geometry=formula.RenderToGeometry(WpfTeXEnvironment.Create(TexStyle.Display,20,"Arial"),Math.Clamp(size,8,96));
            }
            var bounds=geometry.Bounds;
            if(bounds.IsEmpty||!double.IsFinite(bounds.Width)||!double.IsFinite(bounds.Height)||bounds.Width>20000||bounds.Height>4000)return null;
            var group=new DrawingGroup();
            using(var dc=group.Open())
            {
                dc.DrawRectangle(Brushes.Transparent,null,new Rect(0,0,bounds.Width+4,bounds.Height+4));
                dc.PushTransform(new TranslateTransform(2-bounds.X,2-bounds.Y));
                if(halo)dc.DrawGeometry(Brushes.White,new Pen(Brushes.White,size*.14),geometry);
                dc.DrawGeometry(foreground,null,geometry);dc.Pop();
            }
            group.Freeze();var result=new DrawingImage(group);result.Freeze();return result;
        }
        catch(Exception ex) when(ex is not OutOfMemoryException){return null;}
    }
    internal static bool IsBoundedFormula(string value)
    {
        if(value.Length is 0 or >2048)return false;
        var depth=0;var commands=0;
        for(var i=0;i<value.Length;i++)
        {
            if(value[i]=='{'){if(++depth>16)return false;}
            else if(value[i]=='}'){if(--depth<0)return false;}
            else if(value[i]=='\\')
            {
                if(++commands>128||++i==value.Length)return false;
                if(!char.IsAsciiLetter(value[i])){if(!"\\ ,;!{}|".Contains(value[i]))return false;continue;}
                var start=i;while(i+1<value.Length&&char.IsAsciiLetter(value[i+1]))i++;
                if(!Commands.Contains(value[start..(i+1)]))return false;
            }
        }
        return depth==0;
    }
}

/// <summary>A deliberately small arithmetic grammar for legacy plain answers.</summary>
internal sealed class PlainMathNotation
{
    private readonly string _text;private int _index,_depth;
    private PlainMathNotation(string text)=>_text=text;
    internal static bool TryConvert(string text,out string latex)
    {
        latex=text;if(text.Length is 0 or >600||text.IndexOfAny(['\r','\n'])>=0)return false;
        if(!text.Any(c=>"=^/_+−-*/×⁰¹²³⁴⁵⁶⁷⁸⁹".Contains(c))&&!double.TryParse(text,System.Globalization.NumberStyles.AllowDecimalPoint,System.Globalization.CultureInfo.InvariantCulture,out _))return false;
        if(Regex.IsMatch(text,"[0-9]\\s+[0-9]",RegexOptions.CultureInvariant,TimeSpan.FromMilliseconds(50)))return false;
        // Plain prose and units are not interpreted as implicit multiplication.
        if(Regex.IsMatch(text,"[A-Za-z]{3,}",RegexOptions.CultureInvariant,TimeSpan.FromMilliseconds(50)))return false;
        var expanded=new StringBuilder();const string supers="⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁽⁾";const string ordinary="0123456789+-()";
        for(var i=0;i<text.Length;i++)
        {
            if(supers.Contains(text[i]))
            {
                expanded.Append("^{");while(i<text.Length&&supers.IndexOf(text[i]) is var position&&position>=0){expanded.Append(ordinary[position]);i++;}expanded.Append('}');i--;
            }
            else if(!char.IsWhiteSpace(text[i]))expanded.Append(text[i]=='−'?'-':text[i]);
        }
        try
        {
            var parser=new PlainMathNotation(expanded.ToString());var prefix=parser.Take('=')?"= ":"";var result=prefix+parser.Sum();
            while(parser.Take('='))result+=" = "+parser.Sum();
            if(parser._index!=parser._text.Length)return false;latex=result;return true;
        }
        catch(FormatException){return false;}
    }
    private bool Take(char c){if(_index>=_text.Length||_text[_index]!=c)return false;_index++;return true;}
    private string Sum()
    {
        var value=Product();while(_index<_text.Length&&_text[_index] is '+' or '-')value+=" "+_text[_index++]+" "+Product();return value;
    }
    private string Product()
    {
        var value=Power();
        while(_index<_text.Length)
        {
            if(Take('/'))
            {
                value=@"\frac{"+value+"}{"+Power()+"}";
                if(_index<_text.Length&&(char.IsAsciiLetterOrDigit(_text[_index])||_text[_index] is '(' or '['))throw new FormatException();
            }
            else if(Take('*')||Take('×')||Take('·'))value+=@" \cdot "+Power();
            else if(char.IsAsciiLetterOrDigit(_text[_index])||_text[_index] is '(' or '[')value+=" "+Power();
            else break;
        }
        return value;
    }
    private string Power()
    {
        if(++_depth>16)throw new FormatException();
        var sign=Take('-')?"-":Take('+')?"+":"";var value=Atom();
        if(Take('^')||Take('_')){var operation=_text[_index-1];value+=operation+"{"+Power()+"}";}
        _depth--;return sign+value;
    }
    private string Atom()
    {
        if(_index==_text.Length)throw new FormatException();var c=_text[_index++];
        if(c is '(' or '[' or '{')
        {
            var close=c=='('?')':c=='['?']':'}';var inner=Sum();if(!Take(close))throw new FormatException();
            return c=='{'?inner:@"\left"+c+inner+@"\right"+close;
        }
        if(char.IsAsciiLetter(c))return c.ToString();
        if(char.IsAsciiDigit(c)||c=='.')
        {
            var start=_index-1;while(_index<_text.Length&&(char.IsAsciiDigit(_text[_index])||_text[_index]=='.'))_index++;
            var number=_text[start.._index];if(!double.TryParse(number,System.Globalization.NumberStyles.AllowDecimalPoint,System.Globalization.CultureInfo.InvariantCulture,out _))throw new FormatException();return number;
        }
        throw new FormatException();
    }
}
