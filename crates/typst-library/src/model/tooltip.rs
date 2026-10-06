use ecow::EcoString;

use crate::foundations::{Content, elem};

/// Shows a tooltip when hovering over content.
///
/// - In PDF export, the body is covered by an invisible note that holds the
///   tooltip text, like @pdf.annotate with `{visible: false}`. Viewer support
///   varies: some viewers show the text when hovering over the body, others
///   show nothing. Viewers may also offer to edit the note.
/// - In @html[HTML export], the body is wrapped in a `<span>` element whose
///   `title` attribute holds the tooltip text.
///
/// The body is displayed unchanged.
///
/// = Example <example>
/// ```example
/// Good #tooltip(
///   "Adjusting the space between pairs of letters",
/// )[kerning] improves legibility.
/// ```
///
/// = Accessibility <accessibility>
/// Tooltips are not a replacement for text: readers of printed documents,
/// and readers using assistive technology, may not be able to see them. For
/// abbreviations, prefer @abbr, which makes the expansion available to
/// assistive technology.
#[elem(since = "0.16.0", keywords = ["hover", "title"], Locatable)]
pub struct TooltipElem {
    /// The text of the tooltip.
    #[required]
    pub text: EcoString,

    /// The content to show the tooltip for.
    #[required]
    pub body: Content,
}
