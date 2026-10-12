//! Import of reviewer comments and tracked changes from a DOCX file.
//!
//! The importer compares the text of the reviewed file with the text of a
//! fresh export of the current source (see [`crate::map`]). This locates each
//! comment and change in the source, even if Word restructured the file or the
//! source changed slightly since the export. Then:
//!
//! - New comments become `pdf.annotate` calls around the commented text, or at
//!   its start if the text can't be wrapped. Replies become annotations with
//!   `reply-to`, and resolved threads get a reply with `state: "completed"`.
//! - Tracked changes are applied to the source where they affect plain text.
//!   Other changes, such as those inside equations or across markup, become
//!   annotations that describe the suggestion.

use std::io::Read;
use std::ops::Range;
use std::time::Duration;

use ecow::{EcoString, eco_format};
use rustc_hash::FxHashMap;
use typst_html::HtmlDocument;
use typst_library::{World, WorldExt};
use typst_syntax::{FileId, LinkedNode, Side, Source, Span, SyntaxKind, SyntaxMode};

use crate::TYPST_AUTHOR;
use crate::map::{ExportedAnnotation, TextMap};

/// Settings for importing a review.
#[derive(Debug, Default, Clone)]
pub struct ImportOptions {
    /// What to do with tracked changes.
    pub changes: ChangeMode,
}

/// What to do with tracked changes.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Hash)]
pub enum ChangeMode {
    /// Apply changes to plain text and turn the others into annotations.
    #[default]
    Apply,
    /// Turn all changes into annotations, leaving the text as is.
    Annotate,
    /// Only report the changes.
    Ignore,
}

/// An edit of a source file.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Edit {
    /// The file to edit.
    pub id: FileId,
    /// The byte range to replace.
    pub range: Range<usize>,
    /// The replacement text.
    pub text: String,
}

/// The result of importing a review.
#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    /// The edits to make, sorted by file and position, without overlaps.
    pub edits: Vec<Edit>,
    /// What happened to each comment and change.
    pub items: Vec<ReportItem>,
}

/// What happened to a comment or change.
#[derive(Debug, Clone)]
pub struct ReportItem {
    /// `comment`, `reply`, `insertion`, `deletion` or `replacement`.
    pub kind: &'static str,
    /// The reviewer.
    pub author: EcoString,
    /// The comment or a description of the change.
    pub text: EcoString,
    /// What the importer did.
    pub outcome: EcoString,
    /// Where in the source it went.
    pub location: Option<(FileId, usize)>,
}

/// Imports the comments and tracked changes of a reviewed DOCX file.
///
/// `document` must be the current source compiled for DOCX export.
pub fn import_review(
    world: &dyn World,
    document: &HtmlDocument,
    docx: &[u8],
    options: &ImportOptions,
) -> Result<ImportReport, EcoString> {
    let exported = crate::write::write_for_import(document)
        .map_err(|_| EcoString::from("failed to convert the document"))?;
    let reviewed = read_docx(docx)?;
    let aligner = Aligner::new(&reviewed.text, &exported.map.text);
    let mut importer = Importer {
        world,
        map: &exported.map,
        annotations: &exported.annotations,
        aligner,
        edits: Vec::new(),
        items: Vec::new(),
        order: 0,
        labels: FxHashMap::default(),
        new_labels: Vec::new(),
    };
    importer.comments(&reviewed);
    importer.changes(&reviewed, options.changes);
    Ok(importer.finish())
}

// -------------------------------------------------------------------------
// Reading the DOCX file.

/// The parts of a reviewed DOCX file that the importer needs.
#[derive(Debug, Default)]
struct Reviewed {
    /// The text as it was exported: reviewer insertions are left out and
    /// reviewer deletions are kept.
    text: String,
    comments: Vec<ReadComment>,
    changes: Vec<ReadChange>,
}

#[derive(Debug, Default, Clone)]
struct ReadComment {
    author: EcoString,
    date: Option<EcoString>,
    text: EcoString,
    start: Option<usize>,
    end: Option<usize>,
    para_id: Option<EcoString>,
    parent: Option<EcoString>,
    done: bool,
}

#[derive(Debug, Clone)]
struct ReadChange {
    insert: bool,
    author: EcoString,
    /// The byte range in the original text. Empty for insertions.
    range: Range<usize>,
    /// The inserted or deleted text.
    text: String,
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";

fn read_docx(data: &[u8]) -> Result<Reviewed, EcoString> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data))
        .map_err(|err| eco_format!("not a DOCX file ({err})"))?;
    let mut part = |name: &str| -> Option<String> {
        let mut file = zip.by_name(name).ok()?;
        let mut text = String::new();
        file.read_to_string(&mut text).ok()?;
        Some(text)
    };
    let document =
        part("word/document.xml").ok_or("the DOCX file has no document part")?;
    let footnotes = part("word/footnotes.xml");
    let comments = part("word/comments.xml");
    let extended = part("word/commentsExtended.xml");

    let mut reader = Reader::default();
    let doc = parse(&document)?;
    let body = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name((W, "body")))
        .ok_or("the DOCX file has no body")?;
    reader.story(body);

    if let Some(footnotes) = &footnotes {
        let notes = parse(footnotes)?;
        let mut by_id = FxHashMap::default();
        let mut order = vec![];
        for note in notes
            .root_element()
            .children()
            .filter(|n| n.has_tag_name((W, "footnote")))
        {
            if note.attribute((W, "type")).is_some() {
                continue;
            }
            let id = note.attribute((W, "id")).unwrap_or_default().to_string();
            order.push(id.clone());
            by_id.insert(id, note);
        }
        let mut ids = reader.footnote_refs.clone();
        ids.retain(|id| by_id.contains_key(id));
        for id in order {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        for id in ids {
            reader.text.push('\n');
            reader.story(by_id[&id]);
        }
    }

    let mut read = Reviewed {
        text: reader.text,
        changes: reader.changes,
        comments: vec![],
    };

    if let Some(comments) = &comments {
        let doc = parse(comments)?;
        for comment in doc
            .root_element()
            .children()
            .filter(|n| n.has_tag_name((W, "comment")))
        {
            let id: EcoString = comment.attribute((W, "id")).unwrap_or_default().into();
            let mut text = String::new();
            let mut para_id = None;
            for p in comment.descendants().filter(|n| n.has_tag_name((W, "p"))) {
                if !text.is_empty() {
                    text.push('\n');
                }
                para_id = p.attribute((W14, "paraId")).map(EcoString::from);
                for node in p.descendants() {
                    if node.has_tag_name((W, "t")) {
                        text.push_str(node.text().unwrap_or_default());
                    } else if node.has_tag_name((W, "br")) {
                        text.push('\n');
                    }
                }
            }
            let range = reader.comment_ranges.get(&id).copied().unwrap_or_default();
            read.comments.push(ReadComment {
                author: comment.attribute((W, "author")).unwrap_or("Reviewer").into(),
                date: comment.attribute((W, "date")).map(Into::into),
                text: text.trim().into(),
                start: range.0,
                end: range.1,
                para_id,
                parent: None,
                done: false,
            });
        }
    }

    if let Some(extended) = &extended {
        let doc = parse(extended)?;
        for ex in doc
            .root_element()
            .children()
            .filter(|n| n.has_tag_name((W15, "commentEx")))
        {
            let Some(para) = ex.attribute((W15, "paraId")) else { continue };
            let parent = ex.attribute((W15, "paraIdParent"));
            let done = ex.attribute((W15, "done")) == Some("1");
            for comment in &mut read.comments {
                if comment.para_id.as_deref() == Some(para) {
                    comment.parent = parent.map(Into::into);
                    comment.done = done;
                }
            }
        }
    }

    Ok(read)
}

fn parse(text: &str) -> Result<roxmltree::Document<'_>, EcoString> {
    roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() },
    )
    .map_err(|err| eco_format!("failed to parse the DOCX file ({err})"))
}

/// Walks `WordprocessingML` in document order, collecting the original text.
#[derive(Default)]
struct Reader {
    text: String,
    changes: Vec<ReadChange>,
    comment_ranges: FxHashMap<EcoString, (Option<usize>, Option<usize>)>,
    footnote_refs: Vec<String>,
    /// The change of the previous paragraph's mark, which applies to the
    /// separator before the next paragraph.
    mark: Option<(bool, EcoString)>,
    /// Where the current story starts.
    story_start: usize,
}

impl Reader {
    fn story(&mut self, node: roxmltree::Node) {
        self.story_start = self.text.len();
        self.mark = None;
        self.walk(node, None);
    }

    fn walk(&mut self, node: roxmltree::Node, rev: Option<(bool, EcoString)>) {
        for child in node.children().filter(|n| n.is_element()) {
            let ns = child.tag_name().namespace();
            let name = child.tag_name().name();
            if ns == Some(W) {
                match name {
                    "p" => self.paragraph(child, rev.clone()),
                    "r" => self.run(child, rev.clone()),
                    "ins" | "moveTo" | "del" | "moveFrom" => {
                        let author: EcoString =
                            child.attribute((W, "author")).unwrap_or("Reviewer").into();
                        let insert = matches!(name, "ins" | "moveTo");
                        self.walk(child, Some((insert, author)));
                    }
                    "commentRangeStart" => {
                        let id = child.attribute((W, "id")).unwrap_or_default();
                        self.comment_ranges.entry(id.into()).or_default().0 =
                            Some(self.text.len());
                    }
                    "commentRangeEnd" => {
                        let id = child.attribute((W, "id")).unwrap_or_default();
                        self.comment_ranges.entry(id.into()).or_default().1 =
                            Some(self.text.len());
                    }
                    "pPr" | "rPr" | "sectPr" | "tblPr" | "tblGrid" | "trPr" | "tcPr"
                    | "drawing" | "pict" | "object" => {}
                    _ => self.walk(child, rev.clone()),
                }
            } else if ns == Some(M) && name == "t" {
                self.push(child.text().unwrap_or_default(), &rev);
            } else if ns
                == Some("http://schemas.openxmlformats.org/markup-compatibility/2006")
            {
                // Alternate content holds drawings; skip it.
            } else {
                self.walk(child, rev.clone());
            }
        }
    }

    fn paragraph(&mut self, p: roxmltree::Node, rev: Option<(bool, EcoString)>) {
        if self.text.len() > self.story_start {
            match self.mark.take() {
                Some((true, author)) => {
                    let pos = self.text.len();
                    self.add_change(true, author, pos..pos, "\n");
                }
                Some((false, author)) => {
                    let pos = self.text.len();
                    self.text.push('\n');
                    self.add_change(false, author, pos..pos + 1, "\n");
                }
                None => self.text.push('\n'),
            }
        } else {
            self.mark = None;
        }
        self.walk(p, rev);

        // A tracked change of the paragraph mark.
        let mark = p
            .children()
            .find(|n| n.has_tag_name((W, "pPr")))
            .and_then(|ppr| ppr.children().find(|n| n.has_tag_name((W, "rPr"))))
            .and_then(|rpr| {
                rpr.children()
                    .find(|n| n.has_tag_name((W, "ins")) || n.has_tag_name((W, "del")))
            });
        self.mark = mark.and_then(|m| {
            let author: EcoString =
                m.attribute((W, "author")).unwrap_or("Reviewer").into();
            (author != TYPST_AUTHOR).then(|| (m.tag_name().name() == "ins", author))
        });
    }

    fn run(&mut self, r: roxmltree::Node, rev: Option<(bool, EcoString)>) {
        let style = r
            .children()
            .find(|n| n.has_tag_name((W, "rPr")))
            .and_then(|rpr| rpr.children().find(|n| n.has_tag_name((W, "rStyle"))))
            .and_then(|s| s.attribute((W, "val")));
        if matches!(style, Some("FootnoteReference" | "CommentReference")) {
            for child in r.children() {
                if child.has_tag_name((W, "footnoteReference"))
                    && let Some(id) = child.attribute((W, "id"))
                {
                    self.footnote_refs.push(id.into());
                }
            }
            return;
        }
        for child in r.children().filter(|n| n.is_element()) {
            match child.tag_name().name() {
                "t" | "delText" => self.push(child.text().unwrap_or_default(), &rev),
                "tab" => self.push("\t", &rev),
                "br" | "cr" => self.push("\n", &rev),
                "noBreakHyphen" => self.push("\u{2011}", &rev),
                "footnoteReference" => {
                    if let Some(id) = child.attribute((W, "id")) {
                        self.footnote_refs.push(id.into());
                    }
                }
                "commentReference" => {
                    let id = child.attribute((W, "id")).unwrap_or_default();
                    let pos = self.text.len();
                    let entry = self.comment_ranges.entry(id.into()).or_default();
                    entry.0.get_or_insert(pos);
                    entry.1.get_or_insert(pos);
                }
                _ => {}
            }
        }
    }

    fn push(&mut self, text: &str, rev: &Option<(bool, EcoString)>) {
        match rev {
            Some((insert, author)) if author != TYPST_AUTHOR => {
                let pos = self.text.len();
                if *insert {
                    self.add_change(true, author.clone(), pos..pos, text);
                } else {
                    self.text.push_str(text);
                    self.add_change(false, author.clone(), pos..pos + text.len(), text);
                }
            }
            _ => self.text.push_str(text),
        }
    }

    /// Records a change, merging it with the previous one if they're adjacent.
    fn add_change(
        &mut self,
        insert: bool,
        author: EcoString,
        range: Range<usize>,
        text: &str,
    ) {
        if let Some(last) = self.changes.last_mut()
            && last.insert == insert
            && last.author == author
            && last.range.end == range.start
        {
            last.range.end = range.end;
            last.text.push_str(text);
            return;
        }
        self.changes
            .push(ReadChange { insert, author, range, text: text.into() });
    }
}

// -------------------------------------------------------------------------
// Aligning the reviewed text with the exported text.

/// Maps byte offsets in the reviewed text to byte offsets in the exported
/// text.
struct Aligner {
    /// For each char boundary in the reviewed text, the matching boundary in
    /// the exported text, if the char before or after it matched.
    map: FxHashMap<usize, usize>,
    /// The reviewed char boundaries, in order.
    boundaries: Vec<usize>,
}

impl Aligner {
    fn new(reviewed: &str, exported: &str) -> Self {
        let diff = similar::TextDiff::configure()
            .algorithm(similar::Algorithm::Myers)
            .timeout(Duration::from_secs(10))
            .diff_chars(reviewed, exported);

        let old: Vec<usize> = reviewed
            .char_indices()
            .map(|(i, _)| i)
            .chain([reviewed.len()])
            .collect();
        let new: Vec<usize> = exported
            .char_indices()
            .map(|(i, _)| i)
            .chain([exported.len()])
            .collect();
        let mut map = FxHashMap::default();
        for op in diff.ops() {
            if let similar::DiffOp::Equal { old_index, new_index, len } = *op {
                for k in 0..=len {
                    map.insert(old[old_index + k], new[new_index + k]);
                }
            }
        }
        Self { map, boundaries: old }
    }

    /// Maps an offset, looking backwards (for ends) or forwards (for starts)
    /// to the nearest matched boundary if it didn't match itself.
    fn map(&self, offset: usize, forwards: bool) -> Option<usize> {
        if let Some(&m) = self.map.get(&offset) {
            return Some(m);
        }
        let idx = self.boundaries.partition_point(|&b| b < offset);
        if forwards {
            self.boundaries[idx..].iter().find_map(|b| self.map.get(b).copied())
        } else {
            self.boundaries[..idx]
                .iter()
                .rev()
                .find_map(|b| self.map.get(b).copied())
        }
    }
}

// -------------------------------------------------------------------------
// Turning comments and changes into edits.

struct Importer<'a> {
    world: &'a dyn World,
    map: &'a TextMap,
    annotations: &'a [ExportedAnnotation],
    aligner: Aligner,
    edits: Vec<(Edit, usize)>,
    items: Vec<ReportItem>,
    order: usize,
    /// Labels of comments by their paragraph ID, for replies.
    labels: FxHashMap<EcoString, (EcoString, Option<(FileId, usize)>)>,
    /// Labels added to existing annotations.
    new_labels: Vec<Span>,
}

/// Where an annotation was placed: the label and the position after it.
type Placement = (EcoString, Option<(FileId, usize)>);

impl<'a> Importer<'a> {
    fn insert(&mut self, id: FileId, pos: usize, text: String) {
        self.replace(id, pos..pos, text);
    }

    fn replace(&mut self, id: FileId, range: Range<usize>, text: String) {
        self.order += 1;
        self.edits.push((Edit { id, range, text }, self.order));
    }

    fn report(
        &mut self,
        kind: &'static str,
        author: &str,
        text: &str,
        outcome: impl Into<EcoString>,
        location: Option<(FileId, usize)>,
    ) {
        self.items.push(ReportItem {
            kind,
            author: author.into(),
            text: text.into(),
            outcome: outcome.into(),
            location,
        });
    }

    fn comments(&mut self, reviewed: &Reviewed) {
        // Top-level comments first, so that replies find their labels.
        let (roots, replies): (Vec<_>, Vec<_>) =
            reviewed.comments.iter().partition(|c| c.parent.is_none());

        for comment in &roots {
            let existing = self.existing(comment);
            let placement = match existing {
                Some(annotation) => {
                    let placement = self.label_existing(annotation);
                    self.report(
                        "comment",
                        &comment.author,
                        &comment.text,
                        "already in the source",
                        placement.1,
                    );
                    placement
                }
                None => self.new_comment(comment),
            };
            if let Some(para) = &comment.para_id {
                self.labels.insert(para.clone(), placement.clone());
            }
            if comment.done && existing.is_none() {
                self.reply(
                    &placement,
                    &comment.author,
                    None,
                    "Resolved",
                    Some("completed"),
                );
            }
        }

        for comment in &replies {
            let parent = comment.parent.as_ref().unwrap();
            let Some(placement) = self.labels.get(parent).cloned() else {
                self.report(
                    "reply",
                    &comment.author,
                    &comment.text,
                    "skipped: parent comment not found",
                    None,
                );
                continue;
            };
            if self.existing(comment).is_some() {
                self.report(
                    "reply",
                    &comment.author,
                    &comment.text,
                    "already in the source",
                    placement.1,
                );
                continue;
            }
            let location = self.reply(
                &placement,
                &comment.author,
                comment.date.as_deref(),
                &comment.text,
                None,
            );
            let outcome = if location.is_some() {
                "added as a reply"
            } else {
                "skipped: no location"
            };
            self.report("reply", &comment.author, &comment.text, outcome, location);
        }
    }

    /// Finds an annotation in the source that a comment was exported from.
    fn existing(&self, comment: &ReadComment) -> Option<&'a ExportedAnnotation> {
        let annotations: &'a [ExportedAnnotation] = self.annotations;
        annotations.iter().find(|a| {
            let author = a.author.as_deref().unwrap_or("Author");
            let text = comment.text.as_str();
            author == comment.author
                && (a.contents.trim() == text
                    || text
                        .split_once("] ")
                        .is_some_and(|(_, rest)| a.contents.trim() == rest))
        })
    }

    /// Returns the label of an existing annotation, adding one if needed.
    fn label_existing(&mut self, annotation: &ExportedAnnotation) -> Placement {
        let Some(id) = annotation.span.id() else { return ("".into(), None) };
        let Some(range) = self.world.range(annotation.span) else {
            return ("".into(), None);
        };
        if let Some(label) = &annotation.label {
            // Replies go after the label.
            let end = self
                .world
                .source(id)
                .ok()
                .and_then(|s| {
                    let rest = &s.text()[range.end..];
                    let skip = rest.len() - rest.trim_start().len();
                    rest.trim_start()
                        .starts_with(&format!("<{label}>"))
                        .then(|| range.end + skip + label.len() + 2)
                })
                .unwrap_or(range.end);
            return (label.clone(), Some((id, end)));
        }
        let label = make_label(&[
            annotation.contents.as_str(),
            &format!("{:?}", annotation.span),
        ]);
        if !self.new_labels.contains(&annotation.span) {
            self.new_labels.push(annotation.span);
            self.insert(id, range.end, format!("<{label}>"));
        }
        (label, Some((id, range.end)))
    }

    fn new_comment(&mut self, comment: &ReadComment) -> Placement {
        let label = make_label(&[
            &comment.author,
            &comment.text,
            comment.date.as_deref().unwrap_or(""),
        ]);
        let call = annotate_call(
            &comment.author,
            comment.date.as_deref(),
            &comment.text,
            None,
            None,
        );

        let start = comment.start.or(comment.end);
        let end = comment.end.or(comment.start);
        let (Some(start), Some(end)) = (start, end) else {
            self.report(
                "comment",
                &comment.author,
                &comment.text,
                "skipped: comment has no anchor",
                None,
            );
            return (label, None);
        };

        // Try to wrap the commented text.
        if end > start
            && let Some((id, range)) = self.source_range(start, end)
            && let Some(range) = self.wrappable(id, range)
        {
            self.insert(id, range.start, format!("{call}["));
            self.insert(id, range.end, format!("]<{label}>"));
            let location = Some((id, range.end));
            self.report(
                "comment",
                &comment.author,
                &comment.text,
                "added around the commented text",
                Some((id, range.start)),
            );
            return (label, location);
        }

        // Otherwise, place a note at the start.
        match self.point(start) {
            Some((id, pos)) => {
                self.insert(id, pos, format!("{call}<{label}>"));
                self.report(
                    "comment",
                    &comment.author,
                    &comment.text,
                    "added as a note",
                    Some((id, pos)),
                );
                (label, Some((id, pos)))
            }
            None => {
                self.report(
                    "comment",
                    &comment.author,
                    &comment.text,
                    "skipped: no matching source location",
                    None,
                );
                (label, None)
            }
        }
    }

    fn reply(
        &mut self,
        placement: &Placement,
        author: &str,
        date: Option<&str>,
        text: &str,
        state: Option<&str>,
    ) -> Option<(FileId, usize)> {
        let (label, Some((id, pos))) = placement else { return None };
        let call = annotate_call(author, date, text, Some(label), state);
        self.insert(*id, *pos, call);
        Some((*id, *pos))
    }

    fn changes(&mut self, reviewed: &Reviewed, mode: ChangeMode) {
        // Pair deletions with insertions right after them into replacements.
        let mut changes = reviewed.changes.clone();
        changes.sort_by_key(|c| (c.range.start, !c.insert));
        let mut i = 0;
        while i < changes.len() {
            let change = &changes[i];
            let next = changes.get(i + 1);
            let (deleted, inserted, range, author) = match next {
                Some(next)
                    if !change.insert
                        && next.insert
                        && next.author == change.author
                        && next.range.start == change.range.end =>
                {
                    i += 1;
                    (
                        Some(change.text.clone()),
                        Some(next.text.clone()),
                        change.range.clone(),
                        change.author.clone(),
                    )
                }
                _ if change.insert => (
                    None,
                    Some(change.text.clone()),
                    change.range.clone(),
                    change.author.clone(),
                ),
                _ => (
                    Some(change.text.clone()),
                    None,
                    change.range.clone(),
                    change.author.clone(),
                ),
            };
            i += 1;
            self.change(deleted, inserted, range, &author, mode);
        }
    }

    fn change(
        &mut self,
        deleted: Option<String>,
        inserted: Option<String>,
        range: Range<usize>,
        author: &str,
        mode: ChangeMode,
    ) {
        let (kind, description) = match (&deleted, &inserted) {
            (Some(d), Some(i)) => (
                "replacement",
                eco_format!(
                    "“{}” → “{}”",
                    d.trim_end_matches('\n'),
                    i.trim_end_matches('\n')
                ),
            ),
            (Some(d), None) => {
                ("deletion", eco_format!("“{}”", d.trim_end_matches('\n')))
            }
            (None, Some(i)) => {
                ("insertion", eco_format!("“{}”", i.trim_end_matches('\n')))
            }
            (None, None) => return,
        };

        if mode == ChangeMode::Ignore {
            self.report(kind, author, &description, "ignored", None);
            return;
        }

        if mode == ChangeMode::Apply {
            let target = if range.is_empty() {
                self.insertion_point(range.start).map(|(id, pos)| (id, pos..pos))
            } else {
                self.literal_range(range.start, range.end)
            };
            if let Some((id, src)) = target {
                let text = inserted.as_deref().map(escape_markup).unwrap_or_default();
                let location = Some((id, src.start));
                self.replace(id, src, text);
                self.report(kind, author, &description, "applied", location);
                return;
            }
        }

        // Describe the change in a note instead.
        let note = match kind {
            "replacement" => eco_format!("Suggested replacement: {description}"),
            "deletion" => eco_format!("Suggested deletion: {description}"),
            _ => eco_format!("Suggested insertion: {description}"),
        };
        match self.point(range.start) {
            Some((id, pos)) => {
                let call = annotate_call(author, None, &note, None, None);
                self.insert(id, pos, call);
                let outcome = if mode == ChangeMode::Apply {
                    "added as a note (not plain source text)"
                } else {
                    "added as a note"
                };
                self.report(kind, author, &description, outcome, Some((id, pos)));
            }
            None => self.report(
                kind,
                author,
                &description,
                "skipped: no matching source location",
                None,
            ),
        }
    }

    // ---------------------------------------------------------------------
    // Locating source positions.

    /// Maps an offset in the exported text to a source position. `end`
    /// selects the piece that ends at the offset over one that starts there.
    fn source_pos(&self, exported: usize, end: bool) -> Option<(FileId, usize)> {
        let piece = self.map.piece_at(exported, end)?;
        let id = piece.span.id()?;
        let range = self.world.range(piece.span)?;
        let source = self.world.source(id).ok()?;
        let src = source.text().get(range.clone())?;
        let text = &self.map.text[piece.range.clone()];
        let delta = exported - piece.range.start;
        if src == text {
            Some((id, range.start + delta))
        } else if delta == 0 {
            Some((id, range.start))
        } else if exported == piece.range.end {
            Some((id, range.end))
        } else {
            None
        }
    }

    /// Maps a range of the reviewed text to a source range.
    fn source_range(&self, start: usize, end: usize) -> Option<(FileId, Range<usize>)> {
        let s = self.aligner.map(start, true)?;
        let e = self.aligner.map(end, false)?;
        if e <= s {
            return None;
        }
        let (id1, s) = self.source_pos(s, false)?;
        let (id2, e) = self.source_pos(e, true)?;
        (id1 == id2 && s < e).then_some((id1, s..e))
    }

    /// Maps a range of the reviewed text to a source range that consists only
    /// of plain text, so that it can be replaced.
    fn literal_range(&self, start: usize, end: usize) -> Option<(FileId, Range<usize>)> {
        let s = self.aligner.map(start, true)?;
        let e = self.aligner.map(end, false)?;
        if e <= s {
            return None;
        }
        let mut file = None;
        let mut range: Option<Range<usize>> = None;
        let mut covered = s;
        for piece in &self.map.pieces {
            if piece.range.end <= s || piece.range.start >= e {
                continue;
            }
            if piece.range.start > covered {
                // Untracked text, such as a paragraph break, is in between.
                return None;
            }
            let id = piece.span.id()?;
            let src_range = self.world.range(piece.span)?;
            let source = self.world.source(id).ok()?;
            let src = source.text().get(src_range.clone())?;
            let text = &self.map.text[piece.range.clone()];
            let whitespace = src.trim().is_empty() && text.trim().is_empty();
            if src != text && !whitespace {
                return None;
            }
            if *file.get_or_insert(id) != id {
                return None;
            }
            let lo = s.max(piece.range.start) - piece.range.start;
            let hi = e.min(piece.range.end) - piece.range.start;
            let part = if whitespace {
                (if lo == 0 { src_range.start } else { src_range.end })
                    ..(if hi == text.len() { src_range.end } else { src_range.start })
            } else {
                src_range.start + lo..src_range.start + hi
            };
            match &mut range {
                Some(r) if r.end == part.start || whitespace && r.end <= part.start => {
                    r.end = part.end;
                }
                Some(_) => return None,
                None => range = Some(part),
            }
            covered = piece.range.end;
        }
        if covered < e {
            return None;
        }
        Some((file?, range?))
    }

    /// Finds the source position to insert text at.
    fn insertion_point(&self, offset: usize) -> Option<(FileId, usize)> {
        let e = self.aligner.map(offset, false)?;
        let pos = self.source_pos(e, true).or_else(|| self.source_pos(e, false))?;
        let source = self.world.source(pos.0).ok()?;
        in_markup(&source, pos.1).then_some(pos)
    }

    /// Finds a position in markup at or before an offset of the reviewed text,
    /// for placing a note.
    fn point(&self, offset: usize) -> Option<(FileId, usize)> {
        let e = self
            .aligner
            .map(offset, true)
            .or_else(|| self.aligner.map(offset, false))?;
        let found = self
            .source_pos(e, false)
            .or_else(|| self.source_pos(e, true))
            .or_else(|| {
                // Fall back to the nearest earlier piece.
                let idx = self.map.pieces.partition_point(|p| p.range.end <= e);
                self.map.pieces[..idx]
                    .iter()
                    .rev()
                    .find_map(|p| self.source_pos(p.range.end, true))
            })?;
        let source = self.world.source(found.0).ok()?;
        Some((found.0, markup_position(&source, found.1)))
    }

    /// Checks that a source range can be wrapped in a content block, widening
    /// it to whole markup nodes if needed.
    fn wrappable(&self, id: FileId, range: Range<usize>) -> Option<Range<usize>> {
        let source = self.world.source(id).ok()?;
        let root = LinkedNode::new(source.root());
        let first = root.leaf_at(range.start, Side::After)?;
        let last = root.leaf_at(range.end, Side::Before)?;

        // The markup node that contains the start.
        let mut markup = first.clone();
        while markup.kind() != SyntaxKind::Markup {
            markup = markup.parent()?.clone();
        }
        let markup_range = markup.range();
        if range.end > markup_range.end {
            return None;
        }

        // The children of the markup node at both ends.
        let child_of = |mut node: LinkedNode<'_>| -> Option<(SyntaxKind, Range<usize>)> {
            loop {
                let parent = node.parent()?.clone();
                if parent.range() == markup_range && parent.kind() == SyntaxKind::Markup {
                    return Some((node.kind(), node.range()));
                }
                node = parent;
            }
        };
        let (start_kind, start_range) = child_of(first)?;
        let (end_kind, end_range) = child_of(last)?;
        let start =
            if start_kind == SyntaxKind::Text { range.start } else { start_range.start };
        let end = if end_kind == SyntaxKind::Text { range.end } else { end_range.end };

        // Paragraph breaks and line-based markup can't be wrapped.
        for child in markup.children() {
            let r = child.range();
            if r.end <= start || r.start >= end {
                continue;
            }
            if matches!(
                child.kind(),
                SyntaxKind::Parbreak
                    | SyntaxKind::Heading
                    | SyntaxKind::ListItem
                    | SyntaxKind::EnumItem
                    | SyntaxKind::TermItem
                    | SyntaxKind::LineComment
                    | SyntaxKind::BlockComment
            ) {
                return None;
            }
        }
        if source.text()[start..end].contains("\n\n") {
            return None;
        }
        Some(start..end)
    }

    fn finish(mut self) -> ImportReport {
        self.edits.sort_by(|(a, ao), (b, bo)| {
            (a.id.vpath().get_with_slash(), a.range.start, a.range.end, *ao).cmp(&(
                b.id.vpath().get_with_slash(),
                b.range.start,
                b.range.end,
                *bo,
            ))
        });
        let mut edits: Vec<Edit> = vec![];
        for (edit, _) in self.edits {
            if let Some(last) = edits.last()
                && last.id == edit.id
                && edit.range.start < last.range.end
            {
                self.items.push(ReportItem {
                    kind: "conflict",
                    author: "".into(),
                    text: edit.text.clone().into(),
                    outcome: "skipped: overlaps another edit".into(),
                    location: Some((edit.id, edit.range.start)),
                });
                continue;
            }
            edits.push(edit);
        }
        ImportReport { edits, items: self.items }
    }
}

/// Whether a position is in markup mode, where text can be inserted.
fn in_markup(source: &Source, pos: usize) -> bool {
    let root = LinkedNode::new(source.root());
    [Side::After, Side::Before].into_iter().any(|side| {
        root.leaf_at(pos, side).is_some_and(|leaf| {
            matches!(
                leaf.kind(),
                SyntaxKind::Text
                    | SyntaxKind::SpaceNoNewline
                    | SyntaxKind::SpaceWithNewline
                    | SyntaxKind::Markup
            ) && leaf.parent_kind() == Some(SyntaxKind::Markup)
                && leaf.parent_mode() != Some(SyntaxMode::Code)
        })
    })
}

/// Moves a position out of code or math into the surrounding markup.
fn markup_position(source: &Source, pos: usize) -> usize {
    if in_markup(source, pos) {
        return pos;
    }
    let root = LinkedNode::new(source.root());
    let Some(mut node) = root.leaf_at(pos, Side::After) else { return pos };
    while let Some(parent) = node.parent().cloned() {
        if parent.kind() == SyntaxKind::Markup {
            return node.range().end;
        }
        node = parent;
    }
    pos
}

/// Builds a `pdf.annotate` call.
fn annotate_call(
    author: &str,
    date: Option<&str>,
    text: &str,
    reply_to: Option<&str>,
    state: Option<&str>,
) -> String {
    let mut args = vec![];
    if let Some(label) = reply_to {
        args.push(format!("reply-to: <{label}>"));
    }
    args.push(format!("author: {}", typst_string(author)));
    if let Some(date) = date.and_then(typst_datetime) {
        args.push(format!("date: {date}"));
    }
    if let Some(state) = state {
        args.push(format!("state: \"{state}\""));
    }
    args.push(typst_string(text));
    format!("#pdf.annotate({})", args.join(", "))
}

fn typst_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Converts an ISO 8601 date into a Typst `datetime` call.
fn typst_datetime(date: &str) -> Option<String> {
    let (d, t) = date.split_once('T').unwrap_or((date, "00:00:00"));
    let mut d = d.split('-').map(|p| p.parse::<u32>().ok());
    let (year, month, day) = (d.next()??, d.next()??, d.next()??);
    let t = t.trim_end_matches('Z');
    let t = t.split(['+', '.']).next().unwrap_or(t);
    let mut t = t.split(':').map(|p| p.parse::<u32>().ok());
    let (hour, minute, second) = (
        t.next().flatten().unwrap_or(0),
        t.next().flatten().unwrap_or(0),
        t.next().flatten().unwrap_or(0),
    );
    Some(format!(
        "datetime(year: {year}, month: {month}, day: {day}, hour: {hour}, minute: {minute}, second: {second})"
    ))
}

/// Escapes text for insertion into markup.
fn escape_markup(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' | '#' | '$' | '*' | '_' | '`' | '<' | '>' | '@' | '[' | ']' | '~' => {
                out.push('\\');
                out.push(c);
            }
            '/' if matches!(chars.peek(), Some('/' | '*')) => out.push_str("\\/"),
            '\n' => out.push_str("\n\n"),
            c => out.push(c),
        }
    }
    out
}

/// Makes a label name from a hash of the given parts.
fn make_label(parts: &[&str]) -> EcoString {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    eco_format!("docx-{:08x}", hash as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_body(body: &str) -> Reader {
        let xml = format!("<w:document xmlns:w=\"{W}\" xmlns:m=\"{M}\"><w:body>{body}</w:body></w:document>");
        let doc = parse(&xml).unwrap();
        let body = doc.root_element().first_element_child().unwrap();
        let mut reader = Reader::default();
        reader.story(body);
        reader
    }

    #[test]
    fn test_reader_keeps_original_text() {
        let reader = read_body(
            "<w:p><w:r><w:t>The </w:t></w:r>\
             <w:del w:author=\"Ana\"><w:r><w:delText>quick</w:delText></w:r></w:del>\
             <w:ins w:author=\"Ana\"><w:r><w:t>slow</w:t></w:r></w:ins>\
             <w:r><w:t> fox</w:t></w:r></w:p>\
             <w:p><w:commentRangeStart w:id=\"7\"/><w:r><w:t>Next</w:t></w:r>\
             <w:commentRangeEnd w:id=\"7\"/></w:p>",
        );
        assert_eq!(reader.text, "The quick fox\nNext");
        assert_eq!(reader.changes.len(), 2);
        assert!(!reader.changes[0].insert);
        assert_eq!(reader.changes[0].range, 4..9);
        assert!(reader.changes[1].insert);
        assert_eq!(reader.changes[1].range, 9..9);
        assert_eq!(reader.changes[1].text, "slow");
        assert_eq!(reader.comment_ranges["7"], (Some(14), Some(18)));
    }

    #[test]
    fn test_reader_ignores_typst_changes() {
        let reader = read_body(
            "<w:p><w:ins w:author=\"Typst\"><w:r><w:t>new</w:t></w:r></w:ins>\
             <w:del w:author=\"Typst\"><w:r><w:delText>old</w:delText></w:r></w:del></w:p>",
        );
        assert_eq!(reader.text, "newold");
        assert!(reader.changes.is_empty());
    }

    #[test]
    fn test_reader_paragraph_mark_insertion() {
        let reader = read_body(
            "<w:p><w:pPr><w:rPr><w:ins w:author=\"Ana\"/></w:rPr></w:pPr>\
             <w:r><w:t>One</w:t></w:r></w:p><w:p><w:r><w:t>Two</w:t></w:r></w:p>",
        );
        assert_eq!(reader.text, "OneTwo");
        assert_eq!(reader.changes[0].text, "\n");
        assert_eq!(reader.changes[0].range, 3..3);
    }

    #[test]
    fn test_aligner() {
        let aligner = Aligner::new("The quick fox", "The very quick fox!");
        assert_eq!(aligner.map(4, true), Some(9));
        assert_eq!(aligner.map(13, false), Some(18));
    }

    #[test]
    fn test_escape_markup() {
        assert_eq!(escape_markup("a *b* #c"), "a \\*b\\* \\#c");
        assert_eq!(escape_markup("x // y"), "x \\// y");
        assert_eq!(escape_markup("one\ntwo"), "one\n\ntwo");
    }

    #[test]
    fn test_annotate_call() {
        assert_eq!(
            annotate_call("Ana", Some("2026-10-10T09:30:00Z"), "Say \"why\"", Some("l"), None),
            "#pdf.annotate(reply-to: <l>, author: \"Ana\", date: datetime(year: 2026, \
             month: 10, day: 10, hour: 9, minute: 30, second: 0), \"Say \\\"why\\\"\")"
        );
    }
}
