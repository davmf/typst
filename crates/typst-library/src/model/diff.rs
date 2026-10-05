//! Showing the changes between two versions of a document.

use ecow::EcoString;
use rustc_hash::FxHashMap;
use similar::{Algorithm, DiffOp};
use typst_syntax::Span;

use crate::foundations::{
    Content, Element, Module, NativeElement, Packed, Scope, SequenceElem, ShowSet, Smart,
    StyleChain, StyledElem, Styles, elem,
};
use crate::layout::{
    AlignElem, BlockBody, BlockElem, BoxElem, ColumnsElem, GridChild, GridElem, GridItem,
    PadElem,
};
use crate::model::{
    EmphElem, EnumItem, FigureElem, FootnoteBody, FootnoteElem, HeadingElem, LinkElem,
    ListItem, ParElem, ParbreakElem, QuoteElem, StrongElem, TableChild, TableElem,
    TableItem, TermItem,
};
use crate::text::{
    HighlightElem, OverlineElem, SmallcapsElem, StrikeElem, SubElem, SuperElem, TextElem,
    UnderlineElem,
};
use crate::visualize::Color;

/// The `diff` module with the elements that mark changes.
pub fn module() -> Module {
    let mut scope = Scope::deduplicating();
    scope.start_category(crate::Category::Model);
    scope.define_elem::<InsElem>();
    scope.define_elem::<DelElem>();
    Module::new("diff", scope)
}

/// Content that was inserted in a newer version of a document.
///
/// These elements are not written by hand. They are produced when compiling a
/// document with change tracking, e.g. with `typst compile --diff-base
/// <REV>`. By default, inserted content is shown in green and underlined.
///
/// In paged export, the underline is an @underline element, so it can be
/// configured with set rules on `underline`. In HTML export, inserted content
/// becomes an `<ins>` element.
///
/// = Example <example>
/// ```example
/// #show diff.ins: set text(blue)
/// #show diff.ins: underline
/// The #diff.del[old]#diff.ins[new] text.
/// ```
#[elem(title = "Inserted Content", since = "0.16.0", ShowSet)]
pub struct InsElem {
    /// The inserted content.
    #[required]
    pub body: Content,
}

/// Content that was deleted in a newer version of a document.
///
/// These elements are not written by hand. They are produced when compiling a
/// document with change tracking, e.g. with `typst compile --diff-base
/// <REV>`. By default, deleted content is shown in red with a strikethrough.
/// Deleted headings and figures are neither numbered nor outlined.
///
/// In paged export, the strikethrough is a @strike element, so it can be
/// configured with set rules on `strike`. In HTML export, deleted content
/// becomes a `<del>` element.
///
/// = Example <example>
/// ```example
/// #show diff.del: none
/// The #diff.del[old]#diff.ins[new] text.
/// ```
#[elem(title = "Deleted Content", since = "0.16.0", ShowSet)]
pub struct DelElem {
    /// The deleted content.
    #[required]
    pub body: Content,
}

impl ShowSet for Packed<InsElem> {
    fn show_set(&self, _: StyleChain) -> Styles {
        let mut out = Styles::new();
        out.set(TextElem::fill, Color::from_u8(0x00, 0x80, 0x00, 0xFF).into());
        out
    }
}

impl ShowSet for Packed<DelElem> {
    fn show_set(&self, _: StyleChain) -> Styles {
        let mut out = Styles::new();
        out.set(TextElem::fill, Color::from_u8(0xD0, 0x00, 0x00, 0xFF).into());
        out.set(HeadingElem::numbering, None);
        out.set(HeadingElem::outlined, false);
        out.set(HeadingElem::bookmarked, Smart::Custom(false));
        out.set(FigureElem::numbering, None);
        out.set(FigureElem::outlined, false);
        out
    }
}

/// Merges two versions of a document into one that shows the changes from
/// `old` to `new` through [`InsElem`] and [`DelElem`].
///
/// Text is compared word by word. Elements with a content body (headings,
/// list items, strong emphasis, etc.) are compared recursively if nothing but
/// their body changed. All other elements are compared as a whole.
pub fn diff(old: &Content, new: &Content) -> Content {
    Differ::default().diff(old, new)
}

/// Whether a piece of content was inserted or deleted.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum Change {
    Insert,
    Delete,
}

/// A leaf of a flattened content tree.
#[derive(Clone)]
struct Atom {
    kind: AtomKind,
    /// The style maps that apply to the atom, from outermost to innermost.
    styles: Vec<Styles>,
    /// The shapes of `styles`, see [`shape`].
    shapes: Vec<u128>,
    span: Span,
}

#[derive(Clone)]
enum AtomKind {
    /// A word, a run of whitespace, or a single other character.
    Text(EcoString),
    /// Any other element.
    Elem(Content),
}

/// A key by which atoms are compared.
///
/// Styles are not part of the key: Style maps differ between the versions as
/// soon as anything about them changes (e.g. an argument to a template), which
/// would make all content within them look changed.
#[derive(Clone, Eq, PartialEq, Hash)]
enum Key {
    Text(EcoString),
    Elem(usize),
}

/// An atom in the merged output.
struct Item {
    atom: Atom,
    change: Option<Change>,
}

/// Holds equivalence classes for elements across both versions.
///
/// Content cannot be compared through its hash since that includes spans,
/// which differ between the versions as soon as anything in the source file
/// changes.
#[derive(Default)]
struct Differ {
    /// Representatives of element classes, bucketed by element and text.
    elems: FxHashMap<(Element, EcoString), Vec<(Content, usize)>>,
    /// The number of element classes.
    elem_count: usize,
}

impl Differ {
    /// Merges two versions of some content.
    fn diff(&mut self, old: &Content, new: &Content) -> Content {
        if old == new {
            return new.clone();
        }

        let old = self.flatten(old);
        let new = self.flatten(new);
        let mut items = Vec::new();
        self.diff_atoms(&old, &new, &mut items);
        mark(build(&items, 0))
    }

    /// Flattens content into a sequence of atoms.
    fn flatten(&mut self, content: &Content) -> Vec<Atom> {
        let mut atoms = Vec::new();
        flatten_into(content, &mut Vec::new(), &mut Vec::new(), &mut atoms);
        atoms
    }

    /// Determines the class of an element.
    fn elem_class(&mut self, content: &Content) -> usize {
        let bucket =
            self.elems.entry((content.elem(), content.plain_text())).or_default();
        if let Some((_, class)) = bucket.iter().find(|(other, _)| other == content) {
            return *class;
        }
        let class = self.elem_count;
        self.elem_count += 1;
        bucket.push((content.clone(), class));
        class
    }

    /// Determines the key by which an atom is compared.
    fn key(&mut self, atom: &Atom) -> Key {
        match &atom.kind {
            AtomKind::Text(text) => Key::Text(text.clone()),
            AtomKind::Elem(elem) => Key::Elem(self.elem_class(elem)),
        }
    }

    /// Diffs two atom sequences and writes the merged result into `out`.
    fn diff_atoms(&mut self, old: &[Atom], new: &[Atom], out: &mut Vec<Item>) {
        let old_keys: Vec<_> = old.iter().map(|atom| self.key(atom)).collect();
        let new_keys: Vec<_> = new.iter().map(|atom| self.key(atom)).collect();
        let ops = similar::capture_diff_slices(Algorithm::Myers, &old_keys, &new_keys);

        for op in ops {
            let (old_range, new_range) = (op.old_range(), op.new_range());
            let anchor = anchor(new, new_range.start);
            match op {
                DiffOp::Equal { .. } => out.extend(
                    new[new_range]
                        .iter()
                        .map(|atom| Item { atom: atom.clone(), change: None }),
                ),
                DiffOp::Delete { .. } => {
                    deleted(&old[old_range], anchor, out);
                }
                DiffOp::Insert { .. } => inserted(&new[new_range], out),
                DiffOp::Replace { .. } => {
                    self.replace(&old[old_range], &new[new_range], anchor, out);
                }
            }
        }
    }

    /// Handles a replaced range of atoms. Within the range, elements of the
    /// same kind are paired up and compared recursively.
    fn replace(
        &mut self,
        old: &[Atom],
        new: &[Atom],
        anchor: Option<&Atom>,
        out: &mut Vec<Item>,
    ) {
        let key = |atom: &Atom, unique: usize| match &atom.kind {
            AtomKind::Elem(elem) => Ok(elem.elem()),
            AtomKind::Text(_) => Err(unique),
        };
        let old_keys: Vec<_> = old.iter().enumerate().map(|(i, a)| key(a, i)).collect();
        let new_keys: Vec<_> =
            new.iter().enumerate().map(|(i, a)| key(a, old.len() + i)).collect();
        let ops = similar::capture_diff_slices(Algorithm::Myers, &old_keys, &new_keys);

        for op in ops {
            let (old_range, new_range) = (op.old_range(), op.new_range());
            match op {
                DiffOp::Equal { .. } => {
                    for (o, n) in old[old_range].iter().zip(&new[new_range]) {
                        let (AtomKind::Elem(a), AtomKind::Elem(b)) = (&o.kind, &n.kind)
                        else {
                            unreachable!()
                        };
                        if let Some(merged) = self.diff_elem(a, b) {
                            let kind = AtomKind::Elem(merged);
                            let atom = Atom { kind, ..n.clone() };
                            out.push(Item { atom, change: None });
                        } else {
                            deleted(std::slice::from_ref(o), anchor, out);
                            inserted(std::slice::from_ref(n), out);
                        }
                    }
                }
                _ => {
                    deleted(&old[old_range], anchor, out);
                    inserted(&new[new_range], out);
                }
            }
        }
    }

    /// Merges two elements of the same kind if they only differ in their
    /// content bodies.
    fn diff_elem(&mut self, old: &Content, new: &Content) -> Option<Content> {
        macro_rules! bodies {
            ($($elem:ty => $bodies:expr),* $(,)?) => {
                $(if old.is::<$elem>() {
                    return self.diff_bodies::<$elem>(old, new, $bodies);
                })*
            };
        }

        bodies! {
            StrongElem => |e| vec![&mut e.body],
            EmphElem => |e| vec![&mut e.body],
            HeadingElem => |e| vec![&mut e.body],
            ListItem => |e| vec![&mut e.body],
            EnumItem => |e| vec![&mut e.body],
            TermItem => |e| vec![&mut e.term, &mut e.description],
            ParElem => |e| vec![&mut e.body],
            QuoteElem => |e| vec![&mut e.body],
            LinkElem => |e| vec![&mut e.body],
            FootnoteElem => |e| match &mut e.body {
                FootnoteBody::Content(body) => vec![body],
                FootnoteBody::Reference(_) => vec![],
            },
            FigureElem => |e| {
                let caption = e.caption.as_option_mut().as_mut().and_then(Option::as_mut);
                let mut bodies = vec![&mut e.body];
                bodies.extend(caption.map(|c| &mut c.body));
                bodies
            },
            TableElem => |e| {
                e.children
                    .iter_mut()
                    .flat_map(|child| match child {
                        TableChild::Header(header) => header.children.iter_mut(),
                        TableChild::Footer(footer) => footer.children.iter_mut(),
                        TableChild::Item(item) => std::slice::from_mut(item).iter_mut(),
                    })
                    .filter_map(|item| match item {
                        TableItem::Cell(cell) => Some(&mut cell.body),
                        _ => None,
                    })
                    .collect()
            },
            GridElem => |e| {
                e.children
                    .iter_mut()
                    .flat_map(|child| match child {
                        GridChild::Header(header) => header.children.iter_mut(),
                        GridChild::Footer(footer) => footer.children.iter_mut(),
                        GridChild::Item(item) => std::slice::from_mut(item).iter_mut(),
                    })
                    .filter_map(|item| match item {
                        GridItem::Cell(cell) => Some(&mut cell.body),
                        _ => None,
                    })
                    .collect()
            },
            UnderlineElem => |e| vec![&mut e.body],
            OverlineElem => |e| vec![&mut e.body],
            StrikeElem => |e| vec![&mut e.body],
            HighlightElem => |e| vec![&mut e.body],
            SmallcapsElem => |e| vec![&mut e.body],
            SubElem => |e| vec![&mut e.body],
            SuperElem => |e| vec![&mut e.body],
            AlignElem => |e| vec![&mut e.body],
            PadElem => |e| vec![&mut e.body],
            ColumnsElem => |e| vec![&mut e.body],
            BlockElem => |e| match e.body.as_option_mut() {
                Some(Some(BlockBody::Content(body))) => vec![body],
                _ => vec![],
            },
            BoxElem => |e| e.body.as_option_mut().iter_mut().flatten().collect(),
        }

        None
    }

    /// Merges two elements of type `T` if they only differ in the content
    /// bodies returned by `bodies`.
    fn diff_bodies<T: NativeElement>(
        &mut self,
        old: &Content,
        new: &Content,
        bodies: fn(&mut T) -> Vec<&mut Content>,
    ) -> Option<Content> {
        let mut old = old.to_packed::<T>()?.clone();
        let mut new = new.to_packed::<T>()?.clone();

        // Check that the elements are equal apart from the bodies.
        let mut probe = old.clone();
        let (probe_bodies, new_bodies) = (bodies(&mut probe), bodies(&mut new));
        if probe_bodies.len() != new_bodies.len() {
            return None;
        }
        for (a, b) in probe_bodies.into_iter().zip(new_bodies) {
            *a = b.clone();
        }
        if probe.pack() != new.clone().pack() {
            return None;
        }

        for (a, b) in bodies(&mut old).into_iter().zip(bodies(&mut new)) {
            *b = self.diff(a, b);
        }

        Some(new.pack())
    }
}

/// Flattens content into atoms, tracking the style maps along the way.
fn flatten_into(
    content: &Content,
    styles: &mut Vec<Styles>,
    shapes: &mut Vec<u128>,
    out: &mut Vec<Atom>,
) {
    if let Some(sequence) = content.to_packed::<SequenceElem>() {
        for child in &sequence.children {
            flatten_into(child, styles, shapes, out);
        }
    } else if let Some(styled) = content.to_packed::<StyledElem>() {
        styles.push(styled.styles.clone());
        shapes.push(shape(&styled.styles));
        flatten_into(&styled.child, styles, shapes, out);
        styles.pop();
        shapes.pop();
    } else if let Some(text) = content.to_packed::<TextElem>()
        && content.label().is_none()
    {
        for token in tokenize(&text.text) {
            out.push(Atom {
                kind: AtomKind::Text(token.into()),
                styles: styles.clone(),
                shapes: shapes.clone(),
                span: content.span(),
            });
        }
    } else {
        out.push(Atom {
            kind: AtomKind::Elem(content.clone()),
            styles: styles.clone(),
            shapes: shapes.clone(),
            span: content.span(),
        });
    }
}

/// Determines the shape of a style map: Which kinds of styles it contains,
/// but not their values.
///
/// Style maps with the same shape at the same depth are assumed to stem from
/// the same set rules, show rules, or function calls.
fn shape(styles: &Styles) -> u128 {
    let kinds: Vec<_> = styles
        .iter()
        .map(|style| (std::mem::discriminant(style), style.element()))
        .collect();
    typst_utils::hash128(&kinds)
}

/// The atom whose styles deleted atoms at `index` should adopt.
fn anchor(new: &[Atom], index: usize) -> Option<&Atom> {
    index
        .checked_sub(1)
        .and_then(|i| new.get(i))
        .or_else(|| new.get(index))
}

/// Writes inserted atoms.
fn inserted(atoms: &[Atom], out: &mut Vec<Item>) {
    out.extend(atoms.iter().map(|atom| Item {
        atom: atom.clone(),
        change: marked(atom, Change::Insert),
    }));
}

/// Writes deleted atoms.
///
/// Where the deleted atoms have outer style maps of the same shape as the
/// anchor from the new version, they adopt the anchor's maps. This keeps style
/// maps that cover large parts of the document (like from a template)
/// contiguous. Otherwise, deleted content in the middle would split them,
/// which would for instance cause page breaks with page set rules.
fn deleted(atoms: &[Atom], anchor: Option<&Atom>, out: &mut Vec<Item>) {
    for atom in atoms {
        let mut atom = atom.clone();
        if let Some(anchor) = anchor {
            let shared = atom
                .shapes
                .iter()
                .zip(&anchor.shapes)
                .take_while(|(a, b)| a == b)
                .count();
            atom.styles.splice(..shared, anchor.styles[..shared].iter().cloned());
        }

        // A deleted element must not lead to duplicate labels.
        if let AtomKind::Elem(elem) = &mut atom.kind {
            elem.unlabel();
        }

        let change = marked(&atom, Change::Delete);
        out.push(Item { atom, change });
    }
}

/// Determines how to mark an inserted or deleted atom.
///
/// Paragraph breaks are not marked because they would not break paragraphs
/// anymore within an inline marking element. Deleted paragraph breaks are
/// kept so that deleted paragraphs still appear as such.
fn marked(atom: &Atom, change: Change) -> Option<Change> {
    match &atom.kind {
        AtomKind::Elem(elem) if elem.is::<ParbreakElem>() => None,
        _ => Some(change),
    }
}

/// Rebuilds content from merged items, starting at the given style depth.
///
/// Returns pieces of content along with their change. If all children of a
/// styled piece share the same change, the change is lifted outwards so that
/// it can be marked as a whole.
fn build(items: &[Item], depth: usize) -> Vec<(Content, Option<Change>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let item = &items[i];
        let len;
        if let Some(styles) = item.atom.styles.get(depth) {
            len = items[i..]
                .iter()
                .take_while(|it| it.atom.styles.get(depth) == Some(styles))
                .count();
            let children = build(&items[i..i + len], depth + 1);
            let change = children
                .first()
                .and_then(|(_, change)| *change)
                .filter(|&c| children.iter().all(|(_, other)| *other == Some(c)));
            let body = match change {
                Some(_) => Content::sequence(children.into_iter().map(|(c, _)| c)),
                None => mark(children),
            };
            out.push((body.styled_with_map(styles.clone()), change));
        } else if let AtomKind::Elem(elem) = &item.atom.kind {
            len = 1;
            out.push((elem.clone(), item.change));
        } else {
            // Merge adjacent text for proper shaping.
            let mut text = EcoString::new();
            len = items[i..]
                .iter()
                .map_while(|it| match &it.atom.kind {
                    AtomKind::Text(t)
                        if it.atom.styles.len() == depth && it.change == item.change =>
                    {
                        text.push_str(t);
                        Some(())
                    }
                    _ => None,
                })
                .count();
            out.push((TextElem::packed(text).spanned(item.atom.span), item.change));
        }
        i += len;
    }
    out
}

/// Wraps runs of inserted and deleted content into the respective elements.
fn mark(pieces: Vec<(Content, Option<Change>)>) -> Content {
    let mut seq = Vec::with_capacity(pieces.len());
    for run in pieces.chunk_by(|(_, a), (_, b)| a == b) {
        let body = Content::sequence(run.iter().map(|(c, _)| c.clone()));
        seq.push(match run[0].1 {
            None => body,
            Some(Change::Insert) => InsElem::new(body).pack(),
            Some(Change::Delete) => DelElem::new(body).pack(),
        });
    }
    Content::sequence(seq)
}

/// Splits text into words, runs of whitespace, and single other characters.
fn tokenize(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let len = if first.is_alphanumeric() {
            rest.find(|c: char| !c.is_alphanumeric()).unwrap_or(rest.len())
        } else if first.is_whitespace() {
            rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len())
        } else {
            first.len_utf8()
        };
        let (token, tail) = rest.split_at(len);
        rest = tail;
        Some(token)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens: Vec<_> = tokenize("Hello,  wörld! 42x").collect();
        assert_eq!(tokens, ["Hello", ",", "  ", "wörld", "!", " ", "42x"]);
    }
}
