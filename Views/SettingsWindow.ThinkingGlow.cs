// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public sealed partial class SettingsWindow
{
    private readonly CheckBox _thinkingGlowEnabled=new();
    private string _thinkingGlowColor=ThinkingGlowAppearance.DefaultColor;
    private UIElement ThinkingGlowSettings()
    {
        _thinkingGlowColor=ThinkingGlowAppearance.NormalizeColor(_host.Settings.ThinkingGlowColor);
        var panel=new StackPanel{Margin=new Thickness(0,8,0,8)};
        _thinkingGlowEnabled.Content=LocalizationService.T("AI 思考时显示底部呼吸光效","Breathing glow while AI is working");
        _thinkingGlowEnabled.IsChecked=_host.Settings.ThinkingGlowEnabled;
        panel.Children.Add(_thinkingGlowEnabled);
        var preview=new ThinkingGlow{Height=76,VerticalAlignment=VerticalAlignment.Bottom};
        var sample=new Grid{Height=88,ClipToBounds=true};sample.Children.Add(preview);
        sample.Children.Add(new TextBlock{Text=LocalizationService.T("屏幕底部效果预览","Screen edge preview"),HorizontalAlignment=HorizontalAlignment.Center,VerticalAlignment=VerticalAlignment.Center,Foreground=SecondaryBrush,FontSize=12});
        panel.Children.Add(new Border{Background=new SolidColorBrush(Color.FromRgb(241,244,250)),CornerRadius=new CornerRadius(10),Child=sample,Margin=new Thickness(0,4,0,6)});
        var choose=ActionButton(LocalizationService.T("更改光效颜色…","Change glow color…"));
        void Refresh(){choose.IsEnabled=_thinkingGlowEnabled.IsChecked==true;if(choose.IsEnabled)preview.Start(_thinkingGlowColor);else preview.Stop();}
        choose.Click+=(_,_)=>{if(MewuColorDialog.TryChoose(this,ThinkingGlowAppearance.ParseColor(_thinkingGlowColor),out var color,LocalizationService.T("选择光效颜色","Choose glow color"))){_thinkingGlowColor=$"#{color.R:X2}{color.G:X2}{color.B:X2}";Refresh();}};
        _thinkingGlowEnabled.Checked+=(_,_)=>Refresh();_thinkingGlowEnabled.Unchecked+=(_,_)=>Refresh();
        preview.Loaded+=(_,_)=>Refresh();panel.Children.Add(choose);
        panel.Children.Add(Text(LocalizationService.T("保存后生效；完成或取消回答时自动收起。","Applies after saving; hides when the response finishes or is canceled."),true));
        Refresh();return panel;
    }
}
