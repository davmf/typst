//! Conversion of the HTML DOM into `WordprocessingML`.

use std::fmt::Write as _;

use base64::Engine as _;
use ecow::{EcoString, eco_format};
use rustc_hash::{FxHashMap, FxHashSet};
use typst_html::{HtmlDocument, HtmlElement, HtmlFrame, HtmlNode, attr, tag};
use typst_library::diag::SourceResult;
use typst_library::foundations::{Content, Datetime, Value};
use typst_library::introspection::{Location, Tag};
use typst_syntax::Span;

use crate::map::{ExportedAnnotation, TextMap};
use crate::xml::{escape, text_elem};
use crate::{DocxOptions, TYPST_AUTHOR, math};

/// Whether CSS properties make an element or frame block-level. This is a
/// macro because `typst-html` doesn't export the type of the properties.
macro_rules! is_block_css {
    ($css:expr) => {
        $css.iter().any(|p| p.name == "display" && p.value == "block")
    };
}

/// The text width of the page in EMU (16cm), matching `typst-html`'s frame
/// layout width for DOCX.
pub const TEXT_WIDTH_EMU: i64 = 5_760_000;

/// EMUs per point.
const EMU_PER_PT: f64 = 12_700.0;

/// The resolution of rendered frames in pixels per point.
const FRAME_PIXEL_PER_PT: f32 = 4.0;

/// The result of converting the DOM.
pub struct Output {
    /// The contents of `<w:body>`, without the section properties.
    pub body: String,
    /// The paragraphs of each footnote. Footnote `i` has the ID `i + 1`.
    pub footnotes: Vec<String>,
    /// The comments.
    pub comments: Vec<Comment>,
    /// The formats of the lists. List `i` has the numbering ID `i + 1`.
    pub lists: Vec<ListFormat>,
    /// Relationships of the main document part, besides the fixed ones.
    pub rels: Vec<Rel>,
    /// Media files.
    pub media: Vec<Media>,
    /// The exported text: the body followed by each footnote.
    pub map: TextMap,
    /// All `pdf.annotate` calls in the document.
    pub annotations: Vec<ExportedAnnotation>,
}

/// A Word comment.
pub struct Comment {
    pub id: u32,
    pub author: EcoString,
    pub date: Option<EcoString>,
    pub text: EcoString,
    pub para_id: u32,
    pub parent: Option<u32>,
    pub done: bool,
}

/// The format of a list.
pub enum ListFormat {
    Bullet,
    Ordered { start: i64 },
}

/// A relationship of the main document part.
pub struct Rel {
    pub id: EcoString,
    pub kind: &'static str,
    pub target: EcoString,
    pub external: bool,
}

/// A media file.
pub struct Media {
    pub name: EcoString,
    pub data: Vec<u8>,
}

/// Converts the document's DOM.
pub fn write(document: &HtmlDocument, options: &DocxOptions) -> SourceResult<Output> {
    write_impl(document, options, true)
}

/// Converts the document's DOM without rendering images, for the importer,
/// which only needs the text.
pub fn write_for_import(document: &HtmlDocument) -> SourceResult<Output> {
    write_impl(document, &DocxOptions::default(), false)
}

fn write_impl(
    document: &HtmlDocument,
    options: &DocxOptions,
    render: bool,
) -> SourceResult<Output> {
    let root = document.root();
    let body = root
        .children
        .iter()
        .find_map(|node| match node {
            HtmlNode::Element(elem) if elem.tag == tag::body => Some(elem),
            _ => None,
        })
        .unwrap_or(root);

    let mut writer = Writer::new(options);
    writer.render = render;
    writer.collect(&body.children);
    writer.blocks(&body.children, &Props::default());
    writer.finish_story();

    let mut map = std::mem::take(&mut writer.map);
    let mut footnotes = Vec::with_capacity(writer.footnotes.len());
    for (xml, note_map) in std::mem::take(&mut writer.footnotes) {
        map.push_untracked("\n");
        map.append(&note_map);
        footnotes.push(xml);
    }

    Ok(Output {
        body: writer.out,
        footnotes,
        comments: writer.comments,
        lists: writer.lists,
        rels: writer.rels,
        media: writer.media,
        map,
        annotations: writer.annotations,
    })
}

/// Formatting of runs.
#[derive(Debug, Default, Clone)]
struct Props {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    code: bool,
    smallcaps: bool,
    highlight: bool,
    link: bool,
    vert: Option<&'static str>,
    color: Option<EcoString>,
    rev: Option<Rev>,
}

/// A tracked change that a run is part of.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum Rev {
    Ins,
    Del,
}

impl Props {
    fn rpr(&self) -> String {
        let mut out = String::new();
        if self.link {
            out.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
        } else if self.code {
            out.push_str("<w:rStyle w:val=\"VerbatimChar\"/>");
        }
        if self.bold {
            out.push_str("<w:b/><w:bCs/>");
        }
        if self.italic {
            out.push_str("<w:i/><w:iCs/>");
        }
        if self.smallcaps {
            out.push_str("<w:smallCaps/>");
        }
        if self.strike {
            out.push_str("<w:strike/>");
        }
        if let Some(color) = &self.color {
            write!(out, "<w:color w:val=\"{color}\"/>").unwrap();
        }
        if self.highlight {
            out.push_str("<w:highlight w:val=\"yellow\"/>");
        }
        if self.underline {
            out.push_str("<w:u w:val=\"single\"/>");
        }
        if let Some(vert) = self.vert {
            write!(out, "<w:vertAlign w:val=\"{vert}\"/>").unwrap();
        }
        if out.is_empty() { out } else { eco_format!("<w:rPr>{out}</w:rPr>").into() }
    }
}

/// Paragraph settings of the current block context.
#[derive(Debug, Default, Clone)]
struct Block {
    style: Option<&'static str>,
    num: Option<(usize, usize)>,
    indent: Option<i64>,
    center: bool,
}

/// The state of the story being written (the body or a footnote).
#[derive(Default)]
struct Story {
    out: String,
    map: TextMap,
    in_para: bool,
    last_para_end: Option<usize>,
    last_was_table: bool,
    pending: String,
    paras: usize,
    blocks: Vec<Block>,
}

/// An annotation that becomes a Word comment.
struct Annotation {
    author: Option<EcoString>,
    contents: EcoString,
    date: Value,
    state: Option<EcoString>,
}

impl Annotation {
    fn from_content(content: &Content) -> Self {
        let string = |name| match content.get_by_name(name) {
            Ok(Value::Str(s)) => Some(EcoString::from(s.as_str())),
            Ok(other) if !matches!(other, Value::None | Value::Auto) => {
                Some(other.display().plain_text())
            }
            _ => None,
        };
        Self {
            author: string("author"),
            contents: string("contents").unwrap_or_default(),
            date: content.get_by_name("date").unwrap_or(Value::Auto),
            state: string("state"),
        }
    }
}

struct Writer<'a> {
    options: &'a DocxOptions,
    render: bool,
    /// The formatting and end of the last plain run, for merging runs.
    last_run: Option<(String, usize)>,
    out: String,
    map: TextMap,
    in_para: bool,
    last_para_end: Option<usize>,
    last_was_table: bool,
    pending: String,
    paras: usize,
    blocks: Vec<Block>,
    list_depth: usize,
    in_link: bool,
    next_id: u32,
    next_rel: u32,
    next_drawing: u32,
    footnote_bodies: FxHashMap<EcoString, &'a [HtmlNode]>,
    used_footnotes: FxHashSet<EcoString>,
    footnotes: Vec<(String, TextMap)>,
    comments: Vec<Comment>,
    open_comments: FxHashMap<Location, Vec<u32>>,
    replies: FxHashMap<EcoString, Vec<&'a Content>>,
    annotations: Vec<ExportedAnnotation>,
    lists: Vec<ListFormat>,
    rels: Vec<Rel>,
    media: Vec<Media>,
}

impl<'a> Writer<'a> {
    fn new(options: &'a DocxOptions) -> Self {
        Self {
            options,
            render: true,
            last_run: None,
            out: String::new(),
            map: TextMap::default(),
            in_para: false,
            last_para_end: None,
            last_was_table: false,
            pending: String::new(),
            paras: 0,
            blocks: vec![Block::default()],
            list_depth: 0,
            in_link: false,
            next_id: 1,
            next_rel: 1,
            next_drawing: 1,
            footnote_bodies: FxHashMap::default(),
            used_footnotes: FxHashSet::default(),
            footnotes: Vec::new(),
            comments: Vec::new(),
            open_comments: FxHashMap::default(),
            replies: FxHashMap::default(),
            annotations: Vec::new(),
            lists: Vec::new(),
            rels: Vec::new(),
            media: Vec::new(),
        }
    }

    /// Collects footnote bodies and annotations before writing.
    fn collect(&mut self, nodes: &'a [HtmlNode]) {
        for node in nodes {
            match node {
                HtmlNode::Tag(Tag::Start(content, _)) if is_annotation(content) => {
                    let annotation = Annotation::from_content(content);
                    let label = content.label().map(|l| l.resolve().as_str().into());
                    self.annotations.push(ExportedAnnotation {
                        author: annotation.author,
                        contents: annotation.contents,
                        label,
                        span: content.span(),
                    });
                    if let Ok(Value::Label(target)) = content.get_by_name("reply-to") {
                        self.replies
                            .entry(target.resolve().as_str().into())
                            .or_default()
                            .push(content);
                    }
                }
                HtmlNode::Element(elem) => {
                    if elem.tag == tag::section
                        && elem.attrs.get(attr::role).is_some_and(|r| r == "doc-endnotes")
                    {
                        collect_footnotes(elem, &mut self.footnote_bodies);
                    }
                    self.collect(&elem.children);
                }
                _ => {}
            }
        }
    }

    // ---------------------------------------------------------------------
    // Stories and paragraphs.

    /// Saves the current story's state and starts a fresh one.
    fn swap_story(&mut self, story: &mut Story) {
        self.last_run = None;
        std::mem::swap(&mut self.out, &mut story.out);
        std::mem::swap(&mut self.map, &mut story.map);
        std::mem::swap(&mut self.in_para, &mut story.in_para);
        std::mem::swap(&mut self.last_para_end, &mut story.last_para_end);
        std::mem::swap(&mut self.last_was_table, &mut story.last_was_table);
        std::mem::swap(&mut self.pending, &mut story.pending);
        std::mem::swap(&mut self.paras, &mut story.paras);
        std::mem::swap(&mut self.blocks, &mut story.blocks);
    }

    /// Finishes the current story so that pending markup isn't lost.
    fn finish_story(&mut self) {
        self.close_para();
        if !self.pending.is_empty() || self.last_was_table || self.paras == 0 {
            self.open_para();
            self.close_para();
        }
    }

    fn block(&self) -> &Block {
        self.blocks.last().unwrap()
    }

    fn push_block(&mut self, f: impl FnOnce(&mut Block)) {
        let mut block = self.block().clone();
        block.num = None;
        f(&mut block);
        self.blocks.push(block);
    }

    fn pop_block(&mut self) {
        self.blocks.pop();
    }

    fn open_para(&mut self) {
        self.open_para_with("");
    }

    fn open_para_with(&mut self, extra: &str) {
        if self.in_para {
            return;
        }
        if self.paras > 0 || !self.map.text.is_empty() {
            self.map.push_untracked("\n");
        }
        self.in_para = true;
        self.last_was_table = false;
        self.paras += 1;

        let block = self.blocks.last_mut().unwrap();
        let mut ppr = String::new();
        if let Some(style) = block.style {
            write!(ppr, "<w:pStyle w:val=\"{style}\"/>").unwrap();
        }
        if let Some((num, level)) = block.num.take() {
            write!(
                ppr,
                "<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>",
                level.min(8),
                num + 1
            )
            .unwrap();
        } else if let Some(indent) = block.indent {
            write!(ppr, "<w:ind w:left=\"{indent}\"/>").unwrap();
        }
        ppr.push_str(extra);
        if block.center {
            ppr.push_str("<w:jc w:val=\"center\"/>");
        }

        self.out.push_str("<w:p>");
        if !ppr.is_empty() {
            write!(self.out, "<w:pPr>{ppr}</w:pPr>").unwrap();
        }
        let pending = std::mem::take(&mut self.pending);
        self.out.push_str(&pending);
    }

    fn close_para(&mut self) {
        if !self.in_para {
            return;
        }
        self.last_para_end = Some(self.out.len());
        self.out.push_str("</w:p>");
        self.in_para = false;
    }

    /// Writes markup that starts a range, such as a bookmark start.
    fn mark_start(&mut self, xml: &str) {
        if self.in_para {
            self.out.push_str(xml);
        } else {
            self.pending.push_str(xml);
        }
    }

    /// Writes markup that ends a range. Outside of a paragraph, it goes at the
    /// end of the previous paragraph.
    fn mark_end(&mut self, xml: &str) {
        if self.in_para {
            self.out.push_str(xml);
        } else if let Some(pos) = self.last_para_end.filter(|_| self.pending.is_empty()) {
            self.out.insert_str(pos, xml);
            self.last_para_end = Some(pos + xml.len());
        } else {
            self.pending.push_str(xml);
        }
    }

    fn next_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn add_rel(
        &mut self,
        kind: &'static str,
        target: EcoString,
        external: bool,
    ) -> EcoString {
        let id = eco_format!("rIdT{}", self.next_rel);
        self.next_rel += 1;
        self.rels.push(Rel { id: id.clone(), kind, target, external });
        id
    }

    // ---------------------------------------------------------------------
    // Block-level content.

    fn blocks(&mut self, nodes: &'a [HtmlNode], props: &Props) {
        for node in nodes {
            match node {
                HtmlNode::Element(elem) if is_block(elem) => {
                    self.close_para();
                    self.block_elem(elem, props);
                }
                HtmlNode::Frame(frame) if is_block_css!(frame.css) => {
                    self.close_para();
                    self.frame(frame);
                    self.close_para();
                }
                _ => self.inline(std::slice::from_ref(node), props),
            }
        }
        self.close_para();
    }

    /// Writes the children of a container that must end with a paragraph, such
    /// as a table cell.
    fn container(&mut self, nodes: &'a [HtmlNode], props: &Props) {
        let paras = self.paras;
        self.blocks(nodes, props);
        if self.paras == paras || self.last_was_table || !self.pending.is_empty() {
            self.open_para();
            self.close_para();
        }
    }

    fn block_elem(&mut self, elem: &'a HtmlElement, props: &Props) {
        let bookmark = self.bookmark_start(elem);
        let name = elem.tag.resolve();
        let name = name.as_str();
        match name {
            "p" => {
                self.inline(&elem.children, props);
                self.close_para();
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let style = match name {
                    "h1" => "Title",
                    "h2" => "Heading1",
                    "h3" => "Heading2",
                    "h4" => "Heading3",
                    "h5" => "Heading4",
                    _ => "Heading5",
                };
                self.styled_para(elem, props, style);
            }
            "div" if elem.attrs.get(attr::role).is_some_and(|r| r == "heading") => {
                let level = elem
                    .attrs
                    .get(attr::aria_level)
                    .and_then(|l| l.parse::<usize>().ok())
                    .unwrap_or(7);
                let style = match level {
                    ..=7 => "Heading6",
                    8 => "Heading7",
                    9 => "Heading8",
                    _ => "Heading9",
                };
                self.styled_para(elem, props, style);
            }
            "ul" | "ol" => self.list(elem, name == "ol", props),
            "dl" => {
                for child in &elem.children {
                    match child {
                        HtmlNode::Element(item) if item.tag == tag::dt => {
                            self.push_block(|b| b.style = Some("DefinitionTerm"));
                            self.blocks(&item.children, props);
                            self.pop_block();
                        }
                        HtmlNode::Element(item) if item.tag == tag::dd => {
                            self.push_block(|b| {
                                b.style = Some("Definition");
                                b.indent = None;
                            });
                            self.blocks(&item.children, props);
                            self.pop_block();
                        }
                        other => self.blocks(std::slice::from_ref(other), props),
                    }
                }
            }
            "table" => self.table(elem, props),
            "figure" => {
                self.push_block(|b| b.center = true);
                self.blocks(&elem.children, props);
                self.pop_block();
            }
            "figcaption" => self.styled_para(elem, props, "Caption"),
            "blockquote" => {
                self.push_block(|b| b.style = Some("Quote"));
                self.blocks(&elem.children, props);
                self.pop_block();
            }
            "pre" => {
                self.push_block(|b| b.style = Some("SourceCode"));
                self.open_para();
                self.inline(&elem.children, props);
                self.close_para();
                self.pop_block();
            }
            "hr" => {
                self.open_para_with(
                    "<w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" \
                     w:color=\"auto\"/></w:pBdr>",
                );
                self.close_para();
            }
            "section"
                if elem.attrs.get(attr::role).is_some_and(|r| r == "doc-endnotes") => {}
            "math" => {
                self.open_para();
                self.out.push_str("<m:oMathPara><m:oMath>");
                math::write(elem, &mut self.out, &mut self.map);
                self.out.push_str("</m:oMath></m:oMathPara>");
                self.close_para();
            }
            "li" => self.blocks(&elem.children, props),
            _ => self.blocks(&elem.children, props),
        }
        self.bookmark_end(bookmark);
    }

    fn styled_para(&mut self, elem: &'a HtmlElement, props: &Props, style: &'static str) {
        self.push_block(|b| {
            b.style = Some(style);
            b.indent = None;
        });
        self.blocks(&elem.children, props);
        self.pop_block();
    }

    fn list(&mut self, elem: &'a HtmlElement, ordered: bool, props: &Props) {
        let format = if ordered {
            let start = elem
                .attrs
                .get(attr::start)
                .and_then(|s| s.parse().ok())
                .or_else(|| {
                    elem.children.iter().find_map(|child| match child {
                        HtmlNode::Element(li) => {
                            let v = li.attrs.get(attr::value)?;
                            v.parse().ok()
                        }
                        _ => None,
                    })
                })
                .unwrap_or(1);
            ListFormat::Ordered { start }
        } else {
            ListFormat::Bullet
        };
        let num = self.lists.len();
        self.lists.push(format);

        let level = self.list_depth;
        self.list_depth += 1;
        for child in &elem.children {
            match child {
                HtmlNode::Element(li) if li.tag == tag::li => {
                    let bookmark = self.bookmark_start(li);
                    self.push_block(|b| {
                        b.style = Some("ListParagraph");
                        b.indent = Some(720 * (level as i64 + 1));
                    });
                    self.blocks.last_mut().unwrap().num = Some((num, level));
                    self.blocks(&li.children, props);
                    self.pop_block();
                    self.bookmark_end(bookmark);
                }
                other => self.blocks(std::slice::from_ref(other), props),
            }
        }
        self.list_depth -= 1;
    }

    fn table(&mut self, elem: &'a HtmlElement, props: &Props) {
        // Collect the rows, noting which belong to the header.
        let mut rows: Vec<(bool, &'a HtmlElement)> = Vec::new();
        fn collect<'b>(
            elem: &'b HtmlElement,
            header: bool,
            rows: &mut Vec<(bool, &'b HtmlElement)>,
        ) {
            for child in &elem.children {
                let HtmlNode::Element(child) = child else { continue };
                if child.tag == tag::tr {
                    rows.push((header, child));
                } else if child.tag == tag::thead {
                    collect(child, true, rows);
                } else if child.tag == tag::tbody || child.tag == tag::tfoot {
                    collect(child, false, rows);
                }
            }
        }
        collect(elem, false, &mut rows);

        // Place the cells on a grid, accounting for spans.
        struct Placed<'b> {
            cell: &'b HtmlElement,
            row: usize,
            col: usize,
            colspan: usize,
            rowspan: usize,
        }
        let span_attr = |cell: &HtmlElement, attr| {
            cell.attrs
                .get(attr)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1)
        };
        let mut occupied: Vec<Vec<bool>> = vec![];
        let mut placed: Vec<Placed> = vec![];
        let mut columns = 0;
        for (y, (_, row)) in rows.iter().enumerate() {
            let mut x = 0;
            for cell in &row.children {
                let HtmlNode::Element(cell) = cell else { continue };
                if cell.tag != tag::td && cell.tag != tag::th {
                    continue;
                }
                while occupied.get(y).and_then(|r| r.get(x)).copied().unwrap_or(false) {
                    x += 1;
                }
                let colspan = span_attr(cell, attr::colspan);
                let rowspan = span_attr(cell, attr::rowspan).min(rows.len() - y);
                for dy in 0..rowspan {
                    if occupied.len() <= y + dy {
                        occupied.resize(y + dy + 1, vec![]);
                    }
                    let r = &mut occupied[y + dy];
                    if r.len() < x + colspan {
                        r.resize(x + colspan, false);
                    }
                    for dx in 0..colspan {
                        r[x + dx] = true;
                    }
                }
                placed.push(Placed { cell, row: y, col: x, colspan, rowspan });
                x += colspan;
                columns = columns.max(x);
            }
        }
        if columns == 0 {
            return;
        }

        let center = self.block().center;
        let layout = elem.attrs.get(attr::class).is_some_and(|c| c == "typst-grid");
        self.out.push_str(if layout {
            // Grids are used for layout, so they get no borders.
            "<w:tbl><w:tblPr><w:tblW w:w=\"5000\" w:type=\"pct\"/>"
        } else {
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/>\
             <w:tblW w:w=\"5000\" w:type=\"pct\"/>"
        });
        if center {
            self.out.push_str("<w:jc w:val=\"center\"/>");
        }
        self.out.push_str("<w:tblLook w:val=\"04A0\"/></w:tblPr><w:tblGrid>");
        let width = 9070 / columns;
        for _ in 0..columns {
            write!(self.out, "<w:gridCol w:w=\"{width}\"/>").unwrap();
        }
        self.out.push_str("</w:tblGrid>");

        for (y, (header, _)) in rows.iter().enumerate() {
            self.out.push_str("<w:tr>");
            if *header {
                self.out.push_str("<w:trPr><w:tblHeader/></w:trPr>");
            }
            let mut x = 0;
            while x < columns {
                if let Some(cell) = placed.iter().find(|p| p.row == y && p.col == x) {
                    self.out.push_str("<w:tc><w:tcPr>");
                    write!(
                        self.out,
                        "<w:tcW w:w=\"{}\" w:type=\"dxa\"/>",
                        width * cell.colspan
                    )
                    .unwrap();
                    if cell.colspan > 1 {
                        write!(self.out, "<w:gridSpan w:val=\"{}\"/>", cell.colspan)
                            .unwrap();
                    }
                    if cell.rowspan > 1 {
                        self.out.push_str("<w:vMerge w:val=\"restart\"/>");
                    }
                    self.out.push_str("</w:tcPr>");
                    let bookmark = self.bookmark_start(cell.cell);
                    self.push_block(|b| {
                        b.style = None;
                        b.indent = None;
                        b.center = false;
                    });
                    let mut props = props.clone();
                    if cell.cell.tag == tag::th {
                        props.bold = true;
                    }
                    self.container(&cell.cell.children, &props);
                    self.pop_block();
                    self.bookmark_end(bookmark);
                    self.out.push_str("</w:tc>");
                    x += cell.colspan;
                } else if let Some(cell) = placed
                    .iter()
                    .find(|p| p.col == x && p.row < y && y < p.row + p.rowspan)
                {
                    self.out.push_str("<w:tc><w:tcPr>");
                    if cell.colspan > 1 {
                        write!(self.out, "<w:gridSpan w:val=\"{}\"/>", cell.colspan)
                            .unwrap();
                    }
                    self.out.push_str("<w:vMerge/></w:tcPr><w:p/></w:tc>");
                    x += cell.colspan;
                } else {
                    self.out.push_str("<w:tc><w:p/></w:tc>");
                    x += 1;
                }
            }
            self.out.push_str("</w:tr>");
        }
        self.out.push_str("</w:tbl>");
        self.last_was_table = true;
    }

    // ---------------------------------------------------------------------
    // Inline-level content.

    fn inline(&mut self, nodes: &'a [HtmlNode], props: &Props) {
        for node in nodes {
            match node {
                HtmlNode::Text(text, span) => self.text(text, *span, props),
                HtmlNode::Tag(tag) => self.tag(tag),
                HtmlNode::Element(elem) => self.inline_elem(elem, props),
                HtmlNode::Frame(frame) => self.frame(frame),
            }
        }
    }

    fn text(&mut self, text: &str, span: Span, props: &Props) {
        if !self.in_para && text.trim().is_empty() {
            return;
        }
        self.open_para();
        self.map.push(text, span);

        // Extend the previous run if it has the same formatting.
        let rpr = props.rpr();
        const RUN_END: &str = "</w:t></w:r>";
        if props.rev.is_none()
            && !text.contains(['\n', '\t'])
            && let Some((last_rpr, end)) = &self.last_run
            && *end == self.out.len()
            && *last_rpr == rpr
        {
            self.out.truncate(self.out.len() - RUN_END.len());
            crate::xml::escape_into(&mut self.out, text);
            self.out.push_str(RUN_END);
            self.last_run = Some((rpr, self.out.len()));
            return;
        }

        let (open, close) = self.rev_wrapper(props);
        self.out.push_str(&open);
        self.out.push_str("<w:r>");
        self.out.push_str(&rpr);
        let text_tag = if props.rev == Some(Rev::Del) { "w:delText" } else { "w:t" };
        let mut buf = String::new();
        let flush = |out: &mut String, buf: &mut String| {
            if !buf.is_empty() {
                text_elem(out, text_tag, buf);
                buf.clear();
            }
        };
        for c in text.chars() {
            match c {
                '\n' => {
                    flush(&mut self.out, &mut buf);
                    self.out.push_str("<w:br/>");
                }
                '\t' => {
                    flush(&mut self.out, &mut buf);
                    self.out.push_str("<w:tab/>");
                }
                c => buf.push(c),
            }
        }
        flush(&mut self.out, &mut buf);
        self.out.push_str("</w:r>");
        self.out.push_str(&close);
        self.last_run = (props.rev.is_none() && self.out.ends_with(RUN_END))
            .then_some((rpr, self.out.len()));
    }

    fn rev_wrapper(&mut self, props: &Props) -> (String, String) {
        let Some(rev) = props.rev else { return (String::new(), String::new()) };
        let id = self.next_id();
        let name = match rev {
            Rev::Ins => "w:ins",
            Rev::Del => "w:del",
        };
        let mut open = format!("<{name} w:id=\"{id}\" w:author=\"{TYPST_AUTHOR}\"");
        if let Some(date) = &self.options.timestamp {
            write!(open, " w:date=\"{}\"", escape(date)).unwrap();
        }
        open.push('>');
        (open, format!("</{name}>"))
    }

    fn inline_elem(&mut self, elem: &'a HtmlElement, props: &Props) {
        if is_block(elem) || (elem.tag == tag::mathml::math) {
            if elem.tag == tag::mathml::math && !is_block(elem) {
                self.open_para();
                self.out.push_str("<m:oMath>");
                math::write(elem, &mut self.out, &mut self.map);
                self.out.push_str("</m:oMath>");
                return;
            }
            // A block inside inline content. Treat it as a separate block.
            self.close_para();
            self.block_elem(elem, props);
            return;
        }

        let bookmark = self.bookmark_start(elem);
        let mut props = props.clone();
        let name = elem.tag.resolve();
        match name.as_str() {
            "strong" | "b" => props.bold = true,
            "em" | "i" => props.italic = true,
            "code" | "kbd" | "samp" => props.code = true,
            "s" => props.strike = true,
            "u" => props.underline = true,
            "mark" => props.highlight = true,
            "ins" => props.rev = Some(Rev::Ins),
            "del" => props.rev = Some(Rev::Del),
            "sub" => props.vert = Some("subscript"),
            "sup" => {
                if elem.attrs.get(attr::role).is_some_and(|r| r == "doc-noteref")
                    && let Some(target) = footnote_target(elem)
                    && self.footnote(target)
                {
                    self.bookmark_end(bookmark);
                    return;
                }
                if elem.attrs.get(attr::role).is_some_and(|r| r == "doc-backlink") {
                    self.bookmark_end(bookmark);
                    return;
                }
                props.vert = Some("superscript");
            }
            "br" => {
                self.open_para();
                self.map.push_untracked("\n");
                self.out.push_str("<w:r><w:br/></w:r>");
            }
            "typst-pagebreak" => {
                self.open_para();
                self.map.push_untracked("\n");
                self.out.push_str("<w:r><w:br w:type=\"page\"/></w:r>");
            }
            "img" => self.image(elem),
            "a" => {
                if let Some(href) = elem.attrs.get(attr::href)
                    && !self.in_link
                {
                    self.link(elem, href, &props);
                    self.bookmark_end(bookmark);
                    return;
                }
            }
            _ => {}
        }

        for (name, value) in elem.css.iter().map(|p| (p.name, p.value.as_str())) {
            match (name, value) {
                ("color", v) => props.color = css_color(v),
                ("text-decoration", v) if v.contains("underline") => {
                    props.underline = true;
                }
                ("text-decoration", v) if v.contains("line-through") => {
                    props.strike = true;
                }
                ("font-variant-caps", _) => props.smallcaps = true,
                _ => {}
            }
        }
        if let Some(style) = elem.attrs.get(attr::style) {
            for decl in style.split(';') {
                if let Some((k, v)) = decl.split_once(':')
                    && k.trim() == "color"
                {
                    props.color = css_color(v.trim());
                }
            }
        }

        self.inline(&elem.children, &props);
        self.bookmark_end(bookmark);
    }

    fn link(&mut self, elem: &'a HtmlElement, href: &str, props: &Props) {
        self.open_para();
        if let Some(id) = href.strip_prefix('#') {
            write!(
                self.out,
                "<w:hyperlink w:anchor=\"{}\" w:history=\"1\">",
                bookmark_name(id)
            )
            .unwrap();
        } else {
            let rel = self.add_rel(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
                href.into(),
                true,
            );
            write!(self.out, "<w:hyperlink r:id=\"{rel}\" w:history=\"1\">").unwrap();
        }
        let mut props = props.clone();
        props.link = true;
        self.in_link = true;
        self.inline(&elem.children, &props);
        self.in_link = false;
        if !self.in_para {
            // Children closed the paragraph; reopen to balance the element.
            self.open_para();
        }
        self.out.push_str("</w:hyperlink>");
    }

    fn tag(&mut self, tag: &Tag) {
        match tag {
            Tag::Start(content, _) if is_annotation(content) => {
                if matches!(content.get_by_name("reply-to"), Ok(Value::Label(_))) {
                    return;
                }
                let Some(loc) = content.location() else { return };
                let mut ids = vec![];
                let root = self.comment(content, None, &mut ids);
                self.add_replies(content, root, &mut ids);
                let mut xml = String::new();
                for id in &ids {
                    write!(xml, "<w:commentRangeStart w:id=\"{id}\"/>").unwrap();
                }
                self.mark_start(&xml);
                self.open_comments.insert(loc, ids);
            }
            Tag::End(loc, ..) => {
                if let Some(ids) = self.open_comments.remove(loc) {
                    let mut xml = String::new();
                    for id in &ids {
                        write!(xml, "<w:commentRangeEnd w:id=\"{id}\"/>").unwrap();
                    }
                    for id in &ids {
                        write!(
                            xml,
                            "<w:r><w:rPr><w:rStyle w:val=\"CommentReference\"/></w:rPr>\
                             <w:commentReference w:id=\"{id}\"/></w:r>"
                        )
                        .unwrap();
                    }
                    self.mark_end(&xml);
                }
            }
            Tag::Start(..) => {}
        }
    }

    /// Creates a comment for an annotation and returns its index.
    fn comment(
        &mut self,
        content: &Content,
        parent: Option<usize>,
        ids: &mut Vec<u32>,
    ) -> usize {
        let annotation = Annotation::from_content(content);
        let id = self.comments.len() as u32;
        let date = match &annotation.date {
            Value::Datetime(dt) => iso_datetime(dt),
            Value::None => None,
            _ => self.options.timestamp.clone(),
        };
        let mut text = annotation.contents.clone();
        if let Some(state) = &annotation.state
            && state != "completed"
            && state != "accepted"
        {
            text = eco_format!("[{state}] {text}");
        }
        self.comments.push(Comment {
            id,
            author: annotation.author.unwrap_or_else(|| "Author".into()),
            date,
            text,
            para_id: 0x1000_0000 + id,
            parent: parent.map(|p| self.comments[p].para_id),
            done: false,
        });
        if let Some(p) = parent
            && annotation
                .state
                .as_deref()
                .is_some_and(|s| s == "completed" || s == "accepted")
        {
            // Word marks the whole thread as resolved.
            let mut root = p;
            while let Some(pp) = self.comments[root].parent {
                root = self.comments.iter().position(|c| c.para_id == pp).unwrap();
            }
            self.comments[root].done = true;
        }
        ids.push(id);
        id as usize
    }

    /// Adds the replies to an annotation, flattened into one thread.
    fn add_replies(&mut self, content: &Content, root: usize, ids: &mut Vec<u32>) {
        let Some(label) = content.label() else { return };
        let Some(replies) = self.replies.get(label.resolve().as_str()).cloned() else {
            return;
        };
        for reply in replies {
            self.comment(reply, Some(root), ids);
            self.add_replies(reply, root, ids);
        }
    }

    /// Writes a footnote reference and the footnote. Returns false if the
    /// footnote can't be written.
    fn footnote(&mut self, target: &str) -> bool {
        let Some(nodes) = self.footnote_bodies.get(target).copied() else { return false };
        if !self.used_footnotes.insert(target.into()) {
            return false;
        }

        let id = self.footnotes.len() + 1;
        self.footnotes.push((String::new(), TextMap::default()));
        self.open_para();
        write!(
            self.out,
            "<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr>\
             <w:footnoteReference w:id=\"{id}\"/></w:r>"
        )
        .unwrap();

        let in_link = std::mem::replace(&mut self.in_link, false);
        let mut story = Story {
            blocks: vec![Block { style: Some("FootnoteText"), ..Block::default() }],
            pending: "<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr>\
                      <w:footnoteRef/></w:r><w:r><w:rPr><w:rStyle \
                      w:val=\"FootnoteReference\"/></w:rPr><w:t xml:space=\"preserve\"> \
                      </w:t></w:r>"
                .into(),
            ..Story::default()
        };
        self.swap_story(&mut story);
        self.blocks(nodes, &Props::default());
        self.finish_story();
        self.swap_story(&mut story);
        self.in_link = in_link;
        self.footnotes[id - 1] = (story.out, story.map);
        true
    }

    fn bookmark_start(&mut self, elem: &HtmlElement) -> Option<u32> {
        let id = elem.attrs.get(attr::id)?;
        if id.starts_with("loc-") && elem.tag == tag::sup
            || elem.tag == tag::li && self.footnote_bodies.contains_key(id)
        {
            return None;
        }
        let bm = self.next_id();
        let xml =
            format!("<w:bookmarkStart w:id=\"{bm}\" w:name=\"{}\"/>", bookmark_name(id));
        self.mark_start(&xml);
        Some(bm)
    }

    fn bookmark_end(&mut self, bookmark: Option<u32>) {
        if let Some(bm) = bookmark {
            self.mark_end(&format!("<w:bookmarkEnd w:id=\"{bm}\"/>"));
        }
    }

    // ---------------------------------------------------------------------
    // Images.

    fn image(&mut self, elem: &HtmlElement) {
        if !self.render {
            return;
        }
        let Some(src) = elem.attrs.get(attr::src) else { return };
        let Some((mime, data)) = parse_data_url(src) else { return };
        let natural = |attr| {
            elem.attrs
                .get(attr)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| *v > 0.0)
        };
        let (Some(nw), Some(nh)) = (natural(attr::width), natural(attr::height)) else {
            return;
        };

        // Sizes in points.
        let css =
            |name| elem.css.iter().find(|p| p.name == name).map(|p| p.value.clone());
        let max_pt = TEXT_WIDTH_EMU as f64 / EMU_PER_PT;
        let (w, h) = match (
            css("width").and_then(|v| css_length(&v)),
            css("height").and_then(|v| css_length(&v)),
        ) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * nh / nw),
            (None, Some(h)) => (h * nw / nh, h),
            (None, None) => {
                let w = (nw * 0.75).min(max_pt);
                (w, w * nh / nw)
            }
        };

        let alt = elem.attrs.get(attr::alt).cloned().unwrap_or_default();
        let (main, svg) = match mime.as_str() {
            "image/png" | "image/jpeg" | "image/gif" => {
                let ext = mime.trim_start_matches("image/").replace("jpeg", "jpg");
                (self.add_media(&ext, data), None)
            }
            "image/svg+xml" => {
                let Some(png) = rasterize_svg(&data, w, h) else { return };
                let svg = self.add_media("svg", data);
                (self.add_media("png", png), Some(svg))
            }
            _ => {
                let Some(png) = reencode_png(&data) else { return };
                (self.add_media("png", png), None)
            }
        };
        self.drawing(&main, svg.as_deref(), w, h, &alt);
    }

    fn frame(&mut self, frame: &HtmlFrame) {
        if !self.render {
            return;
        }
        let size = frame.inner.size();
        let png = typst_render::render_frame_png(&frame.inner, FRAME_PIXEL_PER_PT);
        let rel = self.add_media("png", png);
        self.drawing(&rel, None, size.x.to_pt(), size.y.to_pt(), "");
    }

    /// Adds a media file and returns the ID of its relationship.
    fn add_media(&mut self, ext: &str, data: Vec<u8>) -> EcoString {
        let name = eco_format!("image{}.{ext}", self.media.len() + 1);
        self.media.push(Media { name: name.clone(), data });
        self.add_rel(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
            eco_format!("media/{name}"),
            false,
        )
    }

    fn drawing(&mut self, rel: &str, svg: Option<&str>, w: f64, h: f64, alt: &str) {
        self.open_para();
        let id = self.next_drawing;
        self.next_drawing += 1;
        let cx = (w * EMU_PER_PT).round().max(1.0) as i64;
        let cy = (h * EMU_PER_PT).round().max(1.0) as i64;
        let alt = escape(alt);
        let ext = match svg {
            Some(svg) => format!(
                "<a:extLst><a:ext uri=\"{{96DAC541-7B7A-43D3-8B79-37D633B846F1}}\">\
                 <asvg:svgBlip xmlns:asvg=\"http://schemas.microsoft.com/office/drawing/2016/SVG/main\" \
                 r:embed=\"{svg}\"/></a:ext></a:extLst>"
            ),
            None => String::new(),
        };
        write!(
            self.out,
            "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
             <wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\
             <wp:docPr id=\"{id}\" name=\"Picture {id}\" descr=\"{alt}\"/>\
             <wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\
             <a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
             <pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"Picture {id}\" descr=\"{alt}\"/>\
             <pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rel}\">{ext}</a:blip>\
             <a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm>\
             <a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
             <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>\
             </a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
        )
        .unwrap();
    }
}

/// Whether the content is a `pdf.annotate` element.
fn is_annotation(content: &Content) -> bool {
    content.elem().name() == "annotate"
}

/// Whether an element is rendered as a block.
fn is_block(elem: &HtmlElement) -> bool {
    let name = elem.tag.resolve();
    let block = matches!(
        name.as_str(),
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "table"
            | "figure"
            | "figcaption"
            | "blockquote"
            | "pre"
            | "section"
            | "div"
            | "nav"
            | "hr"
            | "header"
            | "footer"
            | "main"
            | "article"
            | "aside"
            | "address"
            | "details"
            | "summary"
    );
    block
        || (elem.tag == tag::mathml::math
            && elem.attrs.get(attr::mathml::display).is_some_and(|d| d == "block"))
        || (elem.tag != tag::img && is_block_css!(elem.css))
}

/// Finds the target ID of a footnote reference.
fn footnote_target(sup: &HtmlElement) -> Option<&str> {
    sup.children.iter().find_map(|child| match child {
        HtmlNode::Element(a) if a.tag == tag::a => {
            a.attrs.get(attr::href)?.strip_prefix('#')
        }
        _ => None,
    })
}

/// Collects the footnote entries in an endnotes section.
fn collect_footnotes<'a>(
    elem: &'a HtmlElement,
    bodies: &mut FxHashMap<EcoString, &'a [HtmlNode]>,
) {
    for child in &elem.children {
        let HtmlNode::Element(child) = child else { continue };
        if child.tag == tag::li
            && let Some(id) = child.attrs.get(attr::id)
        {
            bodies.insert(id.clone(), &child.children);
        } else {
            collect_footnotes(child, bodies);
        }
    }
}

/// Turns an HTML ID into a hidden Word bookmark name.
pub fn bookmark_name(id: &str) -> String {
    let mut name = String::from("_t_");
    for c in id.chars() {
        name.push(if c.is_ascii_alphanumeric() { c } else { '_' });
    }
    name.truncate(40);
    name
}

/// Parses a data URL into its MIME type and decoded data.
fn parse_data_url(src: &str) -> Option<(EcoString, Vec<u8>)> {
    let rest = src.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    let data = base64::engine::general_purpose::STANDARD.decode(data).ok()?;
    Some((mime.into(), data))
}

/// Renders an SVG image into a PNG fallback.
fn rasterize_svg(data: &[u8], w: f64, h: f64) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let scale = 2.0 * 96.0 / 72.0;
    let pw = (w * scale).round().max(1.0) as u32;
    let ph = (h * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph)?;
    let size = tree.size();
    let ts = tiny_skia::Transform::from_scale(
        pw as f32 / size.width(),
        ph as f32 / size.height(),
    );
    resvg::render(&tree, ts, &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

/// Converts a raster image that Word may not support into a PNG.
fn reencode_png(data: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(data).ok()?;
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// Parses a CSS length into points. Percentages are relative to the text
/// width.
fn css_length(value: &str) -> Option<f64> {
    let value = value.trim();
    let split = value.find(|c: char| c.is_ascii_alphabetic() || c == '%')?;
    let (num, unit) = value.split_at(split);
    let num: f64 = num.trim().parse().ok()?;
    let text_width = TEXT_WIDTH_EMU as f64 / EMU_PER_PT;
    Some(match unit {
        "pt" => num,
        "px" => num * 0.75,
        "cm" => num * 72.0 / 2.54,
        "mm" => num * 72.0 / 25.4,
        "in" => num * 72.0,
        "em" => num * 11.0,
        "%" => num / 100.0 * text_width,
        _ => return None,
    })
}

/// Parses a CSS color into a hex RGB value.
fn css_color(value: &str) -> Option<EcoString> {
    let hex = value.trim().strip_prefix('#')?;
    let hex = match hex.len() {
        3 | 4 => hex.chars().take(3).flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => hex[..6].to_string(),
        _ => return None,
    };
    Some(hex.to_uppercase().into())
}

/// Formats a datetime as ISO 8601 in UTC, as Word expects.
fn iso_datetime(dt: &Datetime) -> Option<EcoString> {
    Some(eco_format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        dt.year()?,
        dt.month()?,
        dt.day()?,
        dt.hour().unwrap_or(0),
        dt.minute().unwrap_or(0),
        dt.second().unwrap_or(0),
    ))
}
