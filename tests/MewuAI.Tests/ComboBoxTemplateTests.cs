using System.Runtime.ExceptionServices;
using System.Xml.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using Xunit;

namespace MewuAI.Tests;

[CollectionDefinition("WPF theme resources", DisableParallelization = true)]
public sealed class WpfThemeResourceCollection { }

[Collection("WPF theme resources")]
public sealed class ComboBoxTemplateTests
{
    [Fact]
    public void EditableModelSupportsSelectionAndManualInputWithActualTheme()
    {
        Exception? failure = null;
        var thread = new Thread(() =>
        {
            try
            {
                var theme = System.Xml.Linq.XDocument.Load(Path.Combine(AppContext.BaseDirectory, "Fixtures", "LightTheme.xaml.xml"));
                // The test host has no application icon resource; the unrelated Window style is not part of this control test.
                theme.Root!.Elements().Where(e => e.Name.LocalName == "Style" && (string?)e.Attribute("TargetType") == "Window").Remove();
                var resources = (ResourceDictionary)System.Windows.Markup.XamlReader.Parse(theme.ToString());
                var combo = new ComboBox { Resources = resources, IsEditable = true, IsTextSearchEnabled = false, Width = 300 };
                combo.Items.Add("MiniMax-M3");
                combo.Items.Add("MiniMax-M2.7");
                combo.ApplyTemplate();
                combo.Measure(new Size(300, 50));
                combo.Arrange(new Rect(0, 0, 300, 50));
                var editor = Assert.IsType<TextBox>(combo.Template.FindName("PART_EditableTextBox", combo));
                Assert.Equal(Visibility.Visible, editor.Visibility);
                combo.SelectedIndex = 1;
                Assert.Equal("MiniMax-M2.7", combo.Text);
                Assert.Equal("MiniMax-M2.7", editor.Text);
                editor.Text = "manual-model-id";
                Dispatcher.CurrentDispatcher.Invoke(() => { }, DispatcherPriority.Background);
                Assert.Equal("manual-model-id", combo.Text);
                combo.SelectedIndex = 0;
                Assert.Equal("MiniMax-M3", editor.Text);
                combo.IsEditable = false;
                Assert.Equal(Visibility.Hidden, editor.Visibility);
            }
            catch (Exception ex) { failure = ex; }
        }) { IsBackground = true };
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(15)));
        if (failure is not null) ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
