//! Typst's DOCX exporter and review importer.
//!
//! DOCX export goes through the HTML target: the document is compiled to an
//! [`HtmlDocument`] and its DOM is converted to `WordprocessingML`. This keeps
//! the full Typst evaluation, while Word takes care of the layout.
//! `pdf.annotate` notes become Word comments, and `diff.ins` and `diff.del`
//! become tracked changes.
//!
//! The importer reads the comments and tracked changes that reviewers added to
//! an exported DOCX file and turns them into edits of the Typst source. See
//! [`import`].

mod import;
mod map;
mod math;
mod package;
mod write;
mod xml;

pub use self::import::{
    ChangeMode, Edit, ImportOptions, ImportReport, ReportItem, import_review,
};

use ecow::EcoString;
use typst_html::HtmlDocument;
use typst_library::diag::SourceResult;

/// Settings for DOCX export.
#[derive(Debug, Clone)]
pub struct DocxOptions {
    /// What reviewers are allowed to do with the document in Word.
    pub protection: Protection,
    /// The time to use for comments and changes without a date, as an
    /// ISO 8601 string such as `2026-10-10T12:00:00Z`.
    pub timestamp: Option<EcoString>,
}

impl Default for DocxOptions {
    fn default() -> Self {
        Self {
            protection: Protection::TrackedChanges,
            timestamp: None,
        }
    }
}

/// What reviewers are allowed to do with an exported document in Word.
///
/// The protection is written into the document's settings without a password,
/// so it guards against accidental edits but can be turned off in Word.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Protection {
    /// The document can be edited freely.
    None,
    /// Reviewers can only add comments.
    Comments,
    /// Reviewers can comment, and all edits are recorded as tracked changes.
    #[default]
    TrackedChanges,
    /// The document is read-only.
    ReadOnly,
}

/// The author name used for tracked changes that come from Typst itself, such
/// as those produced by `--diff-base`. The importer ignores changes by this
/// author.
pub const TYPST_AUTHOR: &str = "Typst";

/// Export a document into a DOCX file.
pub fn docx(document: &HtmlDocument, options: &DocxOptions) -> SourceResult<Vec<u8>> {
    let output = write::write(document, options)?;
    Ok(package::package(&output, document, options))
}
