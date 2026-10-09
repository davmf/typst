//! Conversion of MathML into Office Math Markup Language (OMML).

use std::fmt::Write;

use typst_html::{HtmlElement, HtmlNode, attr};
use typst_syntax::Span;

use crate::map::TextMap;
use crate::xml::escape_into;

/// Writes the contents of a `<math>` element as the contents of an
/// `<m:oMath>` element.
pub fn write(math: &HtmlElement, out: &mut String, map: &mut TextMap) {
    seq(&args(math), out, map);
}

/// The element and text children of an element, without tags.
fn args(elem: &HtmlElement) -> Vec<&HtmlNode> {
    elem.children
        .iter()
        .filter(|node| match node {
            HtmlNode::Element(_) | HtmlNode::Frame(_) => true,
            HtmlNode::Text(text, _) => !text.trim().is_empty(),
            HtmlNode::Tag(_) => false,
        })
        .collect()
}

/// Writes a sequence of nodes, attaching operands to n-ary operators.
fn seq(nodes: &[&HtmlNode], out: &mut String, map: &mut TextMap) {
    let mut i = 0;
    while i < nodes.len() {
        if let HtmlNode::Element(elem) = nodes[i]
            && let Some(chr) = nary_char(elem)
        {
            // The operand of a sum or integral is the next non-operator node.
            let operand = nodes
                .get(i + 1)
                .filter(|next| !matches!(next, HtmlNode::Element(e) if name(e) == "mo"));
            nary(elem, chr, operand.copied(), out, map);
            i += if operand.is_some() { 2 } else { 1 };
            continue;
        }
        node(nodes[i], out, map);
        i += 1;
    }
}

fn name(elem: &HtmlElement) -> String {
    elem.tag.resolve().as_str().to_string()
}

fn node(node: &HtmlNode, out: &mut String, map: &mut TextMap) {
    match node {
        HtmlNode::Text(text, span) => run(out, map, text, *span, Style::Normal),
        HtmlNode::Element(elem) => element(elem, out, map),
        HtmlNode::Frame(_) | HtmlNode::Tag(_) => {}
    }
}

/// Writes an argument wrapped in the given OMML element.
fn arg(tag: &str, node: Option<&HtmlNode>, out: &mut String, map: &mut TextMap) {
    write!(out, "<{tag}>").unwrap();
    if let Some(node) = node {
        match node {
            HtmlNode::Element(elem) if name(elem) == "mrow" => {
                seq(&args(elem), out, map);
            }
            _ => self::node(node, out, map),
        }
    }
    write!(out, "</{tag}>").unwrap();
}

fn element(elem: &HtmlElement, out: &mut String, map: &mut TextMap) {
    let a = args(elem);
    match name(elem).as_str() {
        "mi" => {
            let (text, span) = token_text(elem);
            let normal = elem
                .attrs
                .get(attr::mathml::mathvariant)
                .is_some_and(|v| v == "normal");
            let (plain, italic) = deitalicize(&text);
            let style = if normal
                || (!italic
                    && plain.chars().count() == 1
                    && plain.chars().all(char::is_alphabetic))
                || plain.chars().count() > 1
            {
                Style::Upright
            } else {
                Style::Normal
            };
            run(out, map, &plain, span, style);
        }
        "mn" | "mo" => {
            let (text, span) = token_text(elem);
            run(out, map, &text, span, Style::Normal);
        }
        "mtext" | "ms" => {
            let (text, span) = token_text(elem);
            run(out, map, &text, span, Style::Text);
        }
        "mspace" | "annotation" | "annotation-xml" | "mprescripts" => {}
        "mfrac" => {
            out.push_str("<m:f>");
            let no_bar = elem.attrs.get(attr::mathml::linethickness).is_some_and(|v| {
                v.trim_end_matches(|c: char| c.is_alphabetic())
                    .parse::<f64>()
                    .is_ok_and(|n| n == 0.0)
            });
            if no_bar {
                out.push_str("<m:fPr><m:type m:val=\"noBar\"/></m:fPr>");
            }
            arg("m:num", a.first().copied(), out, map);
            arg("m:den", a.get(1).copied(), out, map);
            out.push_str("</m:f>");
        }
        "msup" => {
            out.push_str("<m:sSup>");
            arg("m:e", a.first().copied(), out, map);
            arg("m:sup", a.get(1).copied(), out, map);
            out.push_str("</m:sSup>");
        }
        "msub" => {
            out.push_str("<m:sSub>");
            arg("m:e", a.first().copied(), out, map);
            arg("m:sub", a.get(1).copied(), out, map);
            out.push_str("</m:sSub>");
        }
        "msubsup" => {
            out.push_str("<m:sSubSup>");
            arg("m:e", a.first().copied(), out, map);
            arg("m:sub", a.get(1).copied(), out, map);
            arg("m:sup", a.get(2).copied(), out, map);
            out.push_str("</m:sSubSup>");
        }
        "msqrt" => {
            out.push_str(
                "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>",
            );
            seq(&a, out, map);
            out.push_str("</m:e></m:rad>");
        }
        "mroot" => {
            out.push_str("<m:rad>");
            arg("m:deg", a.get(1).copied(), out, map);
            arg("m:e", a.first().copied(), out, map);
            out.push_str("</m:rad>");
        }
        "mover" => {
            let accent =
                elem.attrs.get(attr::mathml::accent).is_some_and(|v| v == "true");
            if accent && let Some(chr) = a.get(1).and_then(|n| single_char(n)) {
                write!(out, "<m:acc><m:accPr><m:chr m:val=\"").unwrap();
                escape_into(out, &chr.to_string());
                out.push_str("\"/></m:accPr>");
                arg("m:e", a.first().copied(), out, map);
                out.push_str("</m:acc>");
            } else {
                out.push_str("<m:limUpp>");
                arg("m:e", a.first().copied(), out, map);
                arg("m:lim", a.get(1).copied(), out, map);
                out.push_str("</m:limUpp>");
            }
        }
        "munder" => {
            out.push_str("<m:limLow>");
            arg("m:e", a.first().copied(), out, map);
            arg("m:lim", a.get(1).copied(), out, map);
            out.push_str("</m:limLow>");
        }
        "munderover" => {
            out.push_str("<m:limLow><m:e><m:limUpp>");
            arg("m:e", a.first().copied(), out, map);
            arg("m:lim", a.get(2).copied(), out, map);
            out.push_str("</m:limUpp></m:e>");
            arg("m:lim", a.get(1).copied(), out, map);
            out.push_str("</m:limLow>");
        }
        "mphantom" => {
            out.push_str("<m:phant><m:e>");
            seq(&a, out, map);
            out.push_str("</m:e></m:phant>");
        }
        "mtable" => table(elem, out, map),
        "semantics" => {
            if let Some(first) = a.first() {
                node(first, out, map);
            }
        }
        "mrow" => {
            if let Some((open, close, inner)) = fenced(&a) {
                out.push_str("<m:d><m:dPr><m:begChr m:val=\"");
                escape_into(out, &open);
                out.push_str("\"/><m:endChr m:val=\"");
                escape_into(out, &close);
                out.push_str("\"/></m:dPr><m:e>");
                seq(inner, out, map);
                out.push_str("</m:e></m:d>");
            } else {
                seq(&a, out, map);
            }
        }
        _ => seq(&a, out, map),
    }
}

/// Writes a table, as an equation array for multi-line equations and as a
/// matrix otherwise.
fn table(elem: &HtmlElement, out: &mut String, map: &mut TextMap) {
    let class = elem.attrs.get(attr::class).cloned().unwrap_or_default();
    let rows: Vec<&HtmlElement> = args(elem)
        .into_iter()
        .filter_map(|n| match n {
            HtmlNode::Element(e) if name(e) == "mtr" => Some(e),
            _ => None,
        })
        .collect();

    if class.contains("multiline-equation") || class.contains("aligned") {
        out.push_str("<m:eqArr>");
        for row in rows {
            out.push_str("<m:e>");
            for (i, cell) in cells(row).into_iter().enumerate() {
                if i > 0 {
                    // Alignment points.
                    run(out, map, "&", Span::detached(), Style::Normal);
                }
                seq(&args(cell), out, map);
            }
            out.push_str("</m:e>");
        }
        out.push_str("</m:eqArr>");
    } else {
        out.push_str("<m:m>");
        for row in rows {
            out.push_str("<m:mr>");
            for cell in cells(row) {
                out.push_str("<m:e>");
                seq(&args(cell), out, map);
                out.push_str("</m:e>");
            }
            out.push_str("</m:mr>");
        }
        out.push_str("</m:m>");
    }
}

/// The cells of a table row.
fn cells(row: &HtmlElement) -> Vec<&HtmlElement> {
    args(row)
        .into_iter()
        .filter_map(|n| match n {
            HtmlNode::Element(e) if name(e) == "mtd" => Some(e),
            _ => None,
        })
        .collect()
}

/// Writes an n-ary operator such as a sum with its limits and operand.
fn nary(
    elem: &HtmlElement,
    chr: char,
    operand: Option<&HtmlNode>,
    out: &mut String,
    map: &mut TextMap,
) {
    let a = args(elem);
    let (under, over) = match name(elem).as_str() {
        "munderover" | "msubsup" => (a.get(1).copied(), a.get(2).copied()),
        "munder" | "msub" => (a.get(1).copied(), None),
        "mover" | "msup" => (None, a.get(1).copied()),
        _ => (None, None),
    };
    let loc = if name(elem).starts_with("mu") || name(elem) == "mover" {
        "undOvr"
    } else {
        "subSup"
    };
    out.push_str("<m:nary><m:naryPr><m:chr m:val=\"");
    escape_into(out, &chr.to_string());
    write!(out, "\"/><m:limLoc m:val=\"{loc}\"/>").unwrap();
    if under.is_none() {
        out.push_str("<m:subHide m:val=\"1\"/>");
    }
    if over.is_none() {
        out.push_str("<m:supHide m:val=\"1\"/>");
    }
    // The operator itself is an attribute, so it isn't part of the text map.
    out.push_str("</m:naryPr>");
    arg("m:sub", under, out, map);
    arg("m:sup", over, out, map);
    arg("m:e", operand, out, map);
    out.push_str("</m:nary>");
}

/// If the element is an n-ary operator with limits or scripts, returns the
/// operator's character.
fn nary_char(elem: &HtmlElement) -> Option<char> {
    let n = name(elem);
    if !matches!(
        n.as_str(),
        "munderover" | "munder" | "mover" | "msubsup" | "msub" | "msup"
    ) {
        return None;
    }
    let base = args(elem).into_iter().next()?;
    let HtmlNode::Element(base) = base else { return None };
    if name(base) != "mo" {
        return None;
    }
    let (text, _) = token_text(base);
    let mut chars = text.chars();
    let c = chars.next()?;
    (chars.next().is_none() && "∑∏∐∫∬∭∮∯∰⋃⋂⋁⋀⨀⨁⨂⨄⨆".contains(c)).then_some(c)
}

/// If a row is wrapped in fence operators, returns them and the inner nodes.
fn fenced<'a, 'b>(
    nodes: &'b [&'a HtmlNode],
) -> Option<(String, String, &'b [&'a HtmlNode])> {
    if nodes.len() < 2 {
        return None;
    }
    let fence = |node: &HtmlNode| match node {
        HtmlNode::Element(e)
            if name(e) == "mo"
                && e.attrs.get(attr::mathml::fence).is_some_and(|v| v == "true") =>
        {
            Some(token_text(e).0)
        }
        _ => None,
    };
    let open = fence(nodes[0])?;
    let close = fence(nodes[nodes.len() - 1])?;
    Some((open, close, &nodes[1..nodes.len() - 1]))
}

/// The text of a token element and the span of its first text node.
fn token_text(elem: &HtmlElement) -> (String, Span) {
    let mut text = String::new();
    let mut span = Span::detached();
    for child in &elem.children {
        match child {
            HtmlNode::Text(t, s) => {
                if span.is_detached() {
                    span = *s;
                }
                text.push_str(t);
            }
            HtmlNode::Element(e) => text.push_str(&token_text(e).0),
            _ => {}
        }
    }
    (text, span)
}

fn single_char(node: &HtmlNode) -> Option<char> {
    let HtmlNode::Element(e) = node else { return None };
    let (text, _) = token_text(e);
    let mut chars = text.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum Style {
    /// OMML's default: italic letters, upright everything else.
    Normal,
    /// Upright math text.
    Upright,
    /// Normal (non-math) text.
    Text,
}

fn run(out: &mut String, map: &mut TextMap, text: &str, span: Span, style: Style) {
    if text.is_empty() {
        return;
    }
    map.push(text, span);
    out.push_str("<m:r>");
    match style {
        Style::Normal => {}
        Style::Upright => out.push_str("<m:rPr><m:sty m:val=\"p\"/></m:rPr>"),
        Style::Text => out.push_str("<m:rPr><m:nor/></m:rPr>"),
    }
    out.push_str(
        "<w:rPr><w:rFonts w:ascii=\"Cambria Math\" w:hAnsi=\"Cambria Math\"/></w:rPr>\
         <m:t xml:space=\"preserve\">",
    );
    escape_into(out, text);
    out.push_str("</m:t></m:r>");
}

/// Maps mathematical italic letters back to plain letters, since OMML
/// italicizes letters by itself. Returns whether any letter was italic.
fn deitalicize(text: &str) -> (String, bool) {
    let mut italic = false;
    let out = text
        .chars()
        .map(|c| {
            let u = c as u32;
            let mapped = match u {
                0x1D434..=0x1D44D => char::from_u32(u - 0x1D434 + 'A' as u32),
                0x1D44E..=0x1D467 => char::from_u32(u - 0x1D44E + 'a' as u32),
                0x210E => Some('h'),
                0x1D6E2..=0x1D6FA => char::from_u32(u - 0x1D6E2 + 0x391),
                0x1D6FC..=0x1D714 => char::from_u32(u - 0x1D6FC + 0x3B1),
                _ => None,
            };
            match mapped {
                Some(m) => {
                    italic = true;
                    m
                }
                None => c,
            }
        })
        .collect();
    (out, italic)
}
