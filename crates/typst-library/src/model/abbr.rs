use ecow::EcoString;

use crate::foundations::{Content, elem};

/// An abbreviation, such as an acronym or initialism, with its expansion.
///
/// The expansion is not displayed. Instead, it is made available to readers
/// and assistive technology (AT):
///
/// - In PDF export, the body is tagged as an abbreviation with the expanded
///   form attached. Screen readers may read the expansion instead of the
///   abbreviation.
/// - In @html[HTML export], the body is wrapped in an `<abbr>` element whose
///   `title` attribute holds the expansion. Browsers typically show it as a
///   tooltip.
///
/// = Example <example>
/// ```example
/// #show abbr: set text(fill: blue)
///
/// Typst exports to
/// #abbr("Portable Document Format")[PDF]
/// and HTML.
/// ```
///
/// = Accessibility <accessibility>
/// Only the first use of an abbreviation is usually spelled out in the text.
/// Marking up later uses with `abbr` lets AT users look up or hear the
/// expansion wherever the abbreviation appears. Glossary packages can use
/// this element to mark up the abbreviations they manage.
#[elem(
    title = "Abbreviation",
    since = "0.16.0",
    keywords = ["acronym", "initialism"],
    Locatable,
    Tagged
)]
pub struct AbbrElem {
    /// The expanded form of the abbreviation.
    ///
    /// This should be written out in full, like
    /// `{"Portable Document Format"}` for "PDF".
    #[required]
    pub expansion: EcoString,

    /// The abbreviation itself.
    #[required]
    pub body: Content,
}
