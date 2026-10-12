//! The text of an exported document, with the source spans it came from.
//!
//! The exporter records the text of every story (the main body and each
//! footnote) as it writes it. The importer reads the same text back from a
//! reviewed DOCX file. Aligning the two lets the importer find the source
//! location of a comment or change without storing anything extra in the
//! DOCX file, which keeps the mapping robust against Word rewriting the file.

use std::ops::Range;

use ecow::EcoString;
use typst_syntax::Span;

/// The text of one story, with the spans of the HTML text nodes it came from.
#[derive(Debug, Default, Clone)]
pub struct TextMap {
    /// The plain text. Paragraph ends and line breaks are `\n`.
    pub text: String,
    /// The pieces of text that came from source text nodes, in order.
    pub pieces: Vec<Piece>,
}

/// A piece of exported text that came from one HTML text node.
#[derive(Debug, Clone)]
pub struct Piece {
    /// The byte range in the story's text.
    pub range: Range<usize>,
    /// The span of the text node.
    pub span: Span,
}

impl TextMap {
    /// Appends text that came from a text node with the given span.
    pub fn push(&mut self, text: &str, span: Span) {
        let start = self.text.len();
        self.text.push_str(text);
        if !span.is_detached() && !text.is_empty() {
            self.pieces.push(Piece { range: start..self.text.len(), span });
        }
    }

    /// Appends text that has no source location, such as a paragraph end.
    pub fn push_untracked(&mut self, text: &str) {
        self.text.push_str(text);
    }

    /// Appends another story.
    pub fn append(&mut self, other: &TextMap) {
        let offset = self.text.len();
        self.text.push_str(&other.text);
        self.pieces.extend(other.pieces.iter().map(|piece| Piece {
            range: piece.range.start + offset..piece.range.end + offset,
            span: piece.span,
        }));
    }

    /// Finds the piece containing the given byte offset, or the one that ends
    /// exactly there if `end` is true.
    pub fn piece_at(&self, offset: usize, end: bool) -> Option<&Piece> {
        let idx = self.pieces.partition_point(|piece| piece.range.end < offset);
        let piece = self.pieces.get(idx)?;
        let inside = if end {
            piece.range.start < offset && offset <= piece.range.end
        } else {
            piece.range.start <= offset && offset < piece.range.end
        };
        if inside {
            Some(piece)
        } else if !end {
            // `partition_point` stops at a piece ending exactly at `offset`.
            self.pieces
                .get(idx + 1)
                .filter(|piece| piece.range.start <= offset && offset < piece.range.end)
        } else {
            None
        }
    }
}

/// An annotation found in the exported document.
#[derive(Debug, Clone)]
pub struct ExportedAnnotation {
    /// The author.
    pub author: Option<EcoString>,
    /// The text of the note.
    pub contents: EcoString,
    /// The label attached to the annotation.
    pub label: Option<EcoString>,
    /// The span of the annotation call.
    pub span: Span,
}
