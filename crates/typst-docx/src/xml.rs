//! Helpers for writing XML.

use std::fmt::Write;

/// Escapes text for use in XML element content and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    escape_into(&mut out, text);
    out
}

/// Escapes text into an existing buffer.
pub fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Control characters other than tab and newline are not allowed in
            // XML 1.0.
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
}

/// Writes a `<w:t>` element with preserved whitespace.
pub fn text_elem(out: &mut String, tag: &str, text: &str) {
    write!(out, "<{tag} xml:space=\"preserve\">").unwrap();
    escape_into(out, text);
    write!(out, "</{tag}>").unwrap();
}

/// The XML declaration for all parts.
pub const DECLARATION: &str =
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// The namespaces declared on the root element of document-like parts.
pub const NAMESPACES: &str = concat!(
    "xmlns:wpc=\"http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas\" ",
    "xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" ",
    "xmlns:o=\"urn:schemas-microsoft-com:office:office\" ",
    "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" ",
    "xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\" ",
    "xmlns:v=\"urn:schemas-microsoft-com:vml\" ",
    "xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" ",
    "xmlns:w10=\"urn:schemas-microsoft-com:office:word\" ",
    "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" ",
    "xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" ",
    "xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\" ",
    "xmlns:wpg=\"http://schemas.microsoft.com/office/word/2010/wordprocessingGroup\" ",
    "xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\" ",
    "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" ",
    "xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\" ",
    "xmlns:a14=\"http://schemas.microsoft.com/office/drawing/2010/main\" ",
    "mc:Ignorable=\"w14 w15\"",
);
