using AgentX.Core;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Documents;
using Avalonia.Media;

namespace AgentX.App;

/// <summary>
/// Renders one feed row's text: Pi-grammar Markdown for assistant narrative
/// (headings/bold/code/lists/links in Pi dark hues) or tinted monospace for
/// tool/system output (diff +/- hunks in green/red). Builds Avalonia inlines
/// from the UI-free <see cref="MdDocument"/> model; unknown constructs were
/// already degraded to literal text by the parser, so nothing is lost here.
/// </summary>
public sealed class MarkdownView : SelectableTextBlock
{
    public static readonly StyledProperty<MdDocument?> MarkdownProperty =
        AvaloniaProperty.Register<MarkdownView, MdDocument?>(nameof(Markdown));

    public static readonly StyledProperty<string?> MonoTextProperty =
        AvaloniaProperty.Register<MarkdownView, string?>(nameof(MonoText));

    public static readonly StyledProperty<bool> DiffTintProperty =
        AvaloniaProperty.Register<MarkdownView, bool>(nameof(DiffTint));

    public MdDocument? Markdown
    {
        get => GetValue(MarkdownProperty);
        set => SetValue(MarkdownProperty, value);
    }

    public string? MonoText
    {
        get => GetValue(MonoTextProperty);
        set => SetValue(MonoTextProperty, value);
    }

    public bool DiffTint
    {
        get => GetValue(DiffTintProperty);
        set => SetValue(DiffTintProperty, value);
    }

    static MarkdownView()
    {
        MarkdownProperty.Changed.AddClassHandler<MarkdownView>((v, _) => v.Rebuild());
        MonoTextProperty.Changed.AddClassHandler<MarkdownView>((v, _) => v.Rebuild());
        DiffTintProperty.Changed.AddClassHandler<MarkdownView>((v, _) => v.Rebuild());
    }

    // B7 treatments: shared live brushes, so a treatment switch repaints
    // already-rendered inlines in place (see AxTheme).
    private static IBrush TextBrush => AxTheme.TextBrush;
    private static IBrush MutedBrush => AxTheme.MutedBrush;
    private static IBrush DimBrush => AxTheme.DimBrush;
    private static IBrush VioletBrush => AxTheme.VioletBrush;
    private static IBrush BlueBrush => AxTheme.BlueBrush;
    private static IBrush GreenBrush => AxTheme.GreenBrush;
    private static IBrush RedBrush => AxTheme.RedBrush;
    private static IBrush YellowBrush => AxTheme.YellowBrush;
    // Bundled faces follow the feed appearance (see AxTheme): read live at
    // every Rebuild, never cached, so an appearance switch + row rebuild
    // picks up the new families with no extra machinery.
    private static FontFamily MonoFamily => AxTheme.MonoFont;
    private static FontFamily VoiceFamily => AxTheme.VoiceFont;

    private void Rebuild()
    {
        Inlines?.Clear();
        Text = null;
        // Adjustable feed type (AxFeed.Sizes): prose rows take the full
        // size in the appearance prose face; technical rows sit one step
        // under with runs in the mono face. Local values override the XAML
        // defaults, so Settings changes apply to existing rows after the
        // standard RebuildConversationList refresh.
        var technical = Markdown is null || Markdown.IsEmpty;
        FontFamily = AxTheme.ProseFont;
        FontSize = technical ? AxTheme.FeedMonoSize : AxTheme.FeedFontSize;
        LineHeight = AxTheme.FeedLineHeight;
        if (!technical)
        {
            RenderMarkdown(Markdown!);
        }
        else if (MonoText is { } mono)
        {
            RenderMono(mono, DiffTint);
        }
    }

    private void RenderMarkdown(MdDocument document)
    {
        var inlines = Inlines!;
        foreach (var block in document.Blocks)
        {
            switch (block.Kind)
            {
                case MdBlockKind.Heading:
                    // Pi headings sit near body size (bold + yellow carry the
                    // hierarchy, not large type). The step now scales with the
                    // adjustable feed size (AxFeed.Sizes, 13-16): body+2 / body+1 /
                    // body+0.5 - exactly 16 / 15 / 14.5 at the 14px default, so the
                    // accepted PR1 look is unchanged and 13px/16px keep hierarchy.
                    var size = AxTheme.FeedFontSize + (block.Level switch { 1 => 2.0, 2 => 1.0, _ => 0.5 });
                    AppendInlines(inlines, block.Inlines, null, size: size, weight: FontWeight.Bold, brush: YellowBrush);
                    inlines.Add(new LineBreak());
                    break;
                case MdBlockKind.CodeFence:
                    foreach (var line in (block.Code ?? string.Empty).Split('\n'))
                    {
                        inlines.Add(MonoRun(line.TrimEnd('\r'), GreenBrush));
                        inlines.Add(new LineBreak());
                    }

                    break;
                case MdBlockKind.List:
                    // Nested items keep their source level: two spaces per
                    // MdBlock.Depth (the MarkdownLite indent stack) per level.
                    var bullet = new string(' ', block.Depth * 2)
                        + (block.Ordered ? $"{block.Index}. " : "• ");
                    inlines.Add(new Run(bullet) { Foreground = VioletBrush });
                    AppendInlines(inlines, block.Inlines, null);
                    inlines.Add(new LineBreak());
                    break;
                case MdBlockKind.Quote:
                    inlines.Add(new Run("▍ ") { Foreground = DimBrush });
                    AppendInlines(inlines, block.Inlines, DimBrush, italic: true);
                    inlines.Add(new LineBreak());
                    break;
                case MdBlockKind.Hr:
                    inlines.Add(new Run(new string('─', 24)) { Foreground = DimBrush });
                    inlines.Add(new LineBreak());
                    break;
                default:
                    AppendInlines(inlines, block.Inlines, null);
                    inlines.Add(new LineBreak());
                    inlines.Add(new LineBreak());
                    break;
            }
        }
    }

    private static void AppendInlines(
        InlineCollection inlines,
        IReadOnlyList<MdInline> items,
        IBrush? @default,
        bool italic = false,
        double? size = null,
        FontWeight? weight = null,
        IBrush? brush = null)
    {
        foreach (var inline in items)
        {
            switch (inline.Kind)
            {
                case MdInlineKind.Bold:
                    inlines.Add(StyledRun(inline.Text, size, weight ?? FontWeight.Bold, brush ?? @default ?? TextBrush, italic));
                    break;
                case MdInlineKind.BoldItalic:
                    inlines.Add(StyledRun(inline.Text, size, weight ?? FontWeight.Bold, brush ?? @default ?? TextBrush, true));
                    break;
                case MdInlineKind.Italic:
                    inlines.Add(StyledRun(inline.Text, size, weight, brush ?? @default ?? TextBrush, true));
                    break;
                case MdInlineKind.Strikethrough:
                    // GFM del: colour stays with the surrounding voice (Pi
                    // strikes without recolouring); the rule carries the meaning.
                    inlines.Add(StyledRun(inline.Text, size, weight, brush ?? @default ?? TextBrush, italic,
                        Avalonia.Media.TextDecorations.Strikethrough));
                    break;
                case MdInlineKind.Code:
                    // Inline code keeps its violet Pi role but inherits the block's
                    // size/weight, so code inside a heading stays heading-sized.
                    inlines.Add(MonoRun(inline.Text, VioletBrush, size, weight));
                    break;
                case MdInlineKind.Link:
                    var link = new Run(inline.Text)
                    {
                        Foreground = brush ?? BlueBrush,
                        TextDecorations = Avalonia.Media.TextDecorations.Underline,
                    };
                    if (size is { } linkSize)
                    {
                        link.FontSize = linkSize;
                    }

                    if (weight is { } linkWeight)
                    {
                        link.FontWeight = linkWeight;
                    }

                    inlines.Add(link);
                    // Pi renders the target muted beside the label (mdLinkUrl);
                    // a bare URL renders once, as the label itself.
                    if (inline.Url is { } url && url != inline.Text)
                    {
                        inlines.Add(new Run($" ({url})") { Foreground = MutedBrush });
                    }

                    break;
                default:
                    inlines.Add(StyledRun(inline.Text, size, weight, brush ?? @default ?? TextBrush, italic));
                    break;
            }
        }
    }

    private void RenderMono(string text, bool diffTint)
    {
        var inlines = Inlines!;
        var lines = text.Split('\n');
        for (var i = 0; i < lines.Length; i++)
        {
            var line = lines[i].TrimEnd('\r');
            IBrush? brush = null;
            if (diffTint)
            {
                brush = MarkdownLite.DiffLineKind(line) switch
                {
                    "add" => GreenBrush,
                    "del" => RedBrush,
                    "hunk" => YellowBrush,
                    _ => null,
                };
            }

            inlines.Add(MonoRun(line, brush));
            if (i + 1 < lines.Length)
            {
                inlines.Add(new LineBreak());
            }
        }
    }

    private static Run StyledRun(
        string text,
        double? size,
        FontWeight? weight,
        IBrush brush,
        bool? italic,
        TextDecorationCollection? decorations = null)
    {
        var run = new Run(text) { Foreground = brush };
        if (decorations is not null)
        {
            run.TextDecorations = decorations;
        }

        if (size.HasValue)
        {
            run.FontSize = size.Value;
        }

        if (weight.HasValue)
        {
            run.FontWeight = weight.Value;
        }

        if (italic == true)
        {
            run.FontStyle = FontStyle.Italic;
            // Commentary voice (Operator: Monaspace Argon; Studio: Geist,
            // same as prose). Quotes and agent asides only — never code.
            run.FontFamily = VoiceFamily;
        }

        return run;
    }

    private static Run MonoRun(string text, IBrush? brush, double? size = null, FontWeight? weight = null)
    {
        var run = new Run(text) { FontFamily = MonoFamily, Foreground = brush ?? TextBrush };
        if (size.HasValue)
        {
            run.FontSize = size.Value;
        }

        if (weight.HasValue)
        {
            run.FontWeight = weight.Value;
        }

        return run;
    }
}
