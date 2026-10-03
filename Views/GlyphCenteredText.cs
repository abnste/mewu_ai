// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Automation.Peers;
using System.Windows.Documents;
using System.Windows.Media;

namespace mewu_ai_Assistant.Views;

/// <summary>Centers visible glyphs rather than the font's asymmetric line box.</summary>
public sealed class GlyphCenteredText : FrameworkElement
{
    private const FrameworkPropertyMetadataOptions TypographyOptions =
        FrameworkPropertyMetadataOptions.Inherits | FrameworkPropertyMetadataOptions.AffectsMeasure |
        FrameworkPropertyMetadataOptions.AffectsRender;

    public static readonly DependencyProperty TextProperty = DependencyProperty.Register(
        nameof(Text), typeof(string), typeof(GlyphCenteredText),
        new FrameworkPropertyMetadata(string.Empty,
            FrameworkPropertyMetadataOptions.AffectsMeasure | FrameworkPropertyMetadataOptions.AffectsRender,
            OnTextChanged));
    public static readonly DependencyProperty FontFamilyProperty = TextElement.FontFamilyProperty.AddOwner(
        typeof(GlyphCenteredText), new FrameworkPropertyMetadata(SystemFonts.MessageFontFamily, TypographyOptions, OnTypographyChanged));
    public static readonly DependencyProperty FontSizeProperty = TextElement.FontSizeProperty.AddOwner(
        typeof(GlyphCenteredText), new FrameworkPropertyMetadata(SystemFonts.MessageFontSize, TypographyOptions, OnTypographyChanged));
    public static readonly DependencyProperty FontStyleProperty = TextElement.FontStyleProperty.AddOwner(
        typeof(GlyphCenteredText), new FrameworkPropertyMetadata(FontStyles.Normal, TypographyOptions, OnTypographyChanged));
    public static readonly DependencyProperty FontWeightProperty = TextElement.FontWeightProperty.AddOwner(
        typeof(GlyphCenteredText), new FrameworkPropertyMetadata(FontWeights.Normal, TypographyOptions, OnTypographyChanged));
    public static readonly DependencyProperty FontStretchProperty = TextElement.FontStretchProperty.AddOwner(
        typeof(GlyphCenteredText), new FrameworkPropertyMetadata(FontStretches.Normal, TypographyOptions, OnTypographyChanged));
    public static readonly DependencyProperty ForegroundProperty = TextElement.ForegroundProperty.AddOwner(
        typeof(GlyphCenteredText), new FrameworkPropertyMetadata(Brushes.Black,
            FrameworkPropertyMetadataOptions.Inherits | FrameworkPropertyMetadataOptions.AffectsRender));

    public string Text { get => (string?)GetValue(TextProperty) ?? string.Empty; set => SetValue(TextProperty, value); }
    public FontFamily FontFamily { get => (FontFamily)GetValue(FontFamilyProperty); set => SetValue(FontFamilyProperty, value); }
    public double FontSize { get => (double)GetValue(FontSizeProperty); set => SetValue(FontSizeProperty, value); }
    public FontStyle FontStyle { get => (FontStyle)GetValue(FontStyleProperty); set => SetValue(FontStyleProperty, value); }
    public FontWeight FontWeight { get => (FontWeight)GetValue(FontWeightProperty); set => SetValue(FontWeightProperty, value); }
    public FontStretch FontStretch { get => (FontStretch)GetValue(FontStretchProperty); set => SetValue(FontStretchProperty, value); }
    public Brush Foreground { get => (Brush)GetValue(ForegroundProperty); set => SetValue(ForegroundProperty, value); }

    private Geometry? _glyphGeometry;
    private Rect _glyphBounds = Rect.Empty;

    private static void OnTypographyChanged(DependencyObject sender, DependencyPropertyChangedEventArgs args) =>
        ((GlyphCenteredText)sender).InvalidateGlyphGeometry();

    private static void OnTextChanged(DependencyObject sender, DependencyPropertyChangedEventArgs args)
    {
        var control = (GlyphCenteredText)sender;
        control.InvalidateGlyphGeometry();
        if (string.IsNullOrEmpty(AutomationProperties.GetName(control)) &&
            AutomationPeer.ListenerExists(AutomationEvents.PropertyChanged) &&
            UIElementAutomationPeer.FromElement(control) is { } peer)
            peer.RaisePropertyChangedEvent(AutomationElementIdentifiers.NameProperty,
                args.OldValue as string ?? string.Empty, args.NewValue as string ?? string.Empty);
    }

    protected override void OnPropertyChanged(DependencyPropertyChangedEventArgs args)
    {
        base.OnPropertyChanged(args);
        if (args.Property == LanguageProperty || args.Property == FlowDirectionProperty)
            InvalidateGlyphGeometry();
    }

    protected override void OnDpiChanged(DpiScale oldDpi, DpiScale newDpi)
    {
        base.OnDpiChanged(oldDpi, newDpi);
        InvalidateGlyphGeometry();
    }

    private void InvalidateGlyphGeometry()
    {
        _glyphGeometry = null;
        _glyphBounds = Rect.Empty;
        InvalidateMeasure();
        InvalidateVisual();
    }

    private void EnsureGlyphGeometry()
    {
        if (_glyphGeometry is not null) return;
        if (string.IsNullOrWhiteSpace(Text))
        {
            _glyphGeometry = Geometry.Empty;
            return;
        }
        CultureInfo culture;
        try { culture = Language.GetSpecificCulture(); }
        catch (InvalidOperationException) { culture = CultureInfo.CurrentUICulture; }
        var formatted = new FormattedText(Text, culture, FlowDirection,
            new Typeface(FontFamily, FontStyle, FontWeight, FontStretch), FontSize, Brushes.Black,
            VisualTreeHelper.GetDpi(this).PixelsPerDip);
        // BuildGeometry measures exactly what we draw, including the resolved
        // fallback font. Font ascent/descent and side bearings are not ink.
        // https://learn.microsoft.com/dotnet/api/system.windows.media.formattedtext.buildgeometry
        _glyphGeometry = formatted.BuildGeometry(new Point());
        _glyphBounds = _glyphGeometry.Bounds;
        _glyphGeometry.Freeze();
    }

    protected override Size MeasureOverride(Size availableSize)
    {
        EnsureGlyphGeometry();
        return _glyphBounds.IsEmpty ? new Size() : _glyphBounds.Size;
    }

    protected override void OnRenderSizeChanged(SizeChangedInfo sizeInfo)
    {
        base.OnRenderSizeChanged(sizeInfo);
        InvalidateVisual();
    }

    protected override void OnRender(DrawingContext drawingContext)
    {
        base.OnRender(drawingContext);
        EnsureGlyphGeometry();
        if (_glyphBounds.IsEmpty || Foreground is null) return;
        var left = (RenderSize.Width - _glyphBounds.Width) / 2 - _glyphBounds.Left;
        var top = (RenderSize.Height - _glyphBounds.Height) / 2 - _glyphBounds.Top;
        drawingContext.PushTransform(new TranslateTransform(left, top));
        drawingContext.DrawGeometry(Foreground, null, _glyphGeometry);
        drawingContext.Pop();
    }

    protected override AutomationPeer OnCreateAutomationPeer() => new GlyphCenteredTextAutomationPeer(this);

    private sealed class GlyphCenteredTextAutomationPeer(GlyphCenteredText owner) : FrameworkElementAutomationPeer(owner)
    {
        protected override string GetClassNameCore() => nameof(GlyphCenteredText);
        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Text;
        protected override string GetNameCore()
        {
            var name = base.GetNameCore();
            return string.IsNullOrEmpty(name) ? ((GlyphCenteredText)Owner).Text : name;
        }
    }
}
