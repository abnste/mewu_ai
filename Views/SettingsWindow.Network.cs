// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

public sealed partial class SettingsWindow
{
    private readonly ComboBox _networkProxyMode=new();
    private readonly TextBox _networkProxyUrl=new();
    private UIElement NetworkProxySettings()
    {
        var panel=new StackPanel();
        foreach(var (mode,zh,en) in new[]{("system","跟随系统","System proxy"),("direct","直接连接","Direct connection"),("custom","自定义代理","Custom proxy")})
        {
            var item=new ComboBoxItem{Content=LocalizationService.T(zh,en),Tag=mode};
            _networkProxyMode.Items.Add(item);
            if(mode==_host.Settings.NetworkProxyMode)_networkProxyMode.SelectedItem=item;
        }
        if(_networkProxyMode.SelectedItem is null)_networkProxyMode.SelectedIndex=0;
        _networkProxyUrl.Text=_host.Settings.NetworkProxyUrl;
        panel.Children.Add(AiSettingsForm.Field(LocalizationService.T("API 网络代理","API network proxy"),_networkProxyMode));
        panel.Children.Add(AiSettingsForm.Field(LocalizationService.T("代理地址（不含账号密码）","Proxy URL (without credentials)"),_networkProxyUrl));
        panel.Children.Add(Text(LocalizationService.T("保存后对新的 API 请求和模型目录刷新生效。","Applies to new API requests and model discovery after saving."),true));
        return panel;
    }
}
