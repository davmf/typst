use std::sync::Arc;

use indexmap::IndexMap;
use krilla::annotation::{Annotation, AnnotationHandle, TextAnnotation};
use krilla::geom as kg;
use krilla::num::NormalizedF32;
use krilla::page::Page;
use rustc_hash::{FxBuildHasher, FxHashMap};
use typst_layout::PagedDocument;
use typst_library::foundations::{NativeElement, Packed, Smart, StyleChain};
use typst_library::introspection::{Introspector, Location};
use typst_library::layout::Size;
use typst_library::model::{AnnotationKind, FrameAnnotation, NoteIcon};

use crate::convert::{FrameContext, GlobalContext};
use crate::format::{AnnotateElem, ReviewState};
use crate::link::bounding_box;
use crate::metadata::{convert_datetime, creation_date};
use crate::paint::convert_solid;

/// The annotations on a page, other than links, keyed by the location of the
/// element that produced them.
pub(crate) type PendingAnnotations = IndexMap<Location, PendingAnnotation, FxBuildHasher>;

/// An annotation and the regions it covers on a page.
pub(crate) struct PendingAnnotation {
    spec: Arc<FrameAnnotation>,
    rects: Vec<kg::Rect>,
}

pub(crate) fn handle_annotation(
    fc: &mut FrameContext,
    annotation: &Arc<FrameAnnotation>,
    size: Size,
) {
    let rect = bounding_box(fc, size);
    fc.annotations
        .entry(annotation.location)
        .or_insert_with(|| PendingAnnotation { spec: annotation.clone(), rects: vec![] })
        .rects
        .push(rect);
}

/// Add the annotations found in the page frame.
pub(crate) fn add_annotations(
    gc: &GlobalContext,
    page: &mut Page,
    annotations: PendingAnnotations,
) {
    for (_, pending) in annotations {
        let spec = &pending.spec;
        let rects = merge_rects(pending.rects);
        let first = match spec.kind {
            AnnotationKind::Note if !spec.visible => {
                // Invisible notes are tooltips: cover each line of the body.
                if spec.contents.trim().is_empty() {
                    continue;
                }
                let mut first = None;
                for rect in rects {
                    let text = TextAnnotation::new(rect).with_invisible(true);
                    let handle = page.add_annotation(build(gc, spec, text));
                    first.get_or_insert((handle, rect));
                }
                first
            }
            AnnotationKind::Note => {
                // The rect is the space reserved for the icon.
                let Some(&rect) = rects.first() else { continue };
                let text = TextAnnotation::new(rect)
                    .with_icon(convert_icon(spec.icon))
                    .with_open(spec.open);
                Some((page.add_annotation(build(gc, spec, text)), rect))
            }
        };

        // Replies to tooltips that span several lines reply to the first one.
        if let Some((handle, rect)) = first {
            add_replies(gc, page, spec, spec.location, handle, rect);
        }
    }
}

/// Adds the replies to the annotation at `location`, and their replies.
///
/// Replies share the rect and look of the annotation at the root of their
/// thread, so viewers without support for replies draw them in the same
/// place.
fn add_replies(
    gc: &GlobalContext,
    page: &mut Page,
    root: &FrameAnnotation,
    location: Location,
    handle: AnnotationHandle,
    rect: kg::Rect,
) {
    let Some(replies) = gc.annotation_replies.get(&location) else { return };
    for reply in replies {
        let reply_location = reply.location().unwrap();
        let mut spec = reply.resolve(reply_location, StyleChain::default());
        spec.color = spec.color.or_else(|| root.color.clone());
        let text = TextAnnotation::new(rect)
            .with_invisible(!root.visible)
            .with_icon(convert_icon(root.icon));
        let state = reply.state.get(StyleChain::default()).map(convert_state);
        let annotation = build(gc, &spec, text)
            .with_in_reply_to(Some(handle))
            .with_review_state(state);
        let reply_handle = page.add_annotation(annotation);
        add_replies(gc, page, root, reply_location, reply_handle, rect);
    }
}

/// Replies to annotations, keyed by the location of the annotation they reply
/// to, in document order.
pub(crate) type Replies = FxHashMap<Location, Vec<Packed<AnnotateElem>>>;

/// Collects the replies in the document.
///
/// Replies whose target doesn't exist or isn't an annotation are skipped. The
/// show rule of `pdf.annotate` has already reported them as errors.
pub(crate) fn collect_replies(document: &PagedDocument) -> Replies {
    let introspector = document.introspector();
    let mut replies = Replies::default();
    for elem in introspector.query(&AnnotateElem::ELEM.select()) {
        let elem = elem.into_packed::<AnnotateElem>().unwrap();
        let Some(label) = elem.reply_to.get(StyleChain::default()) else { continue };
        let Ok(target) = introspector.query_label(label) else { continue };
        if target.is::<AnnotateElem>() {
            replies.entry(target.location().unwrap()).or_default().push(elem);
        }
    }
    replies
}

fn convert_state(state: ReviewState) -> krilla::annotation::ReviewState {
    use krilla::annotation::ReviewState as K;
    match state {
        ReviewState::Accepted => K::Accepted,
        ReviewState::Rejected => K::Rejected,
        ReviewState::Cancelled => K::Cancelled,
        ReviewState::Completed => K::Completed,
    }
}

/// Builds a krilla annotation with the properties shared by all kinds.
fn build(gc: &GlobalContext, spec: &FrameAnnotation, text: TextAnnotation) -> Annotation {
    let date = match spec.date {
        Smart::Auto => creation_date(gc),
        Smart::Custom(Some(date)) => convert_datetime(date, None),
        Smart::Custom(None) => None,
    };
    let opacity = NormalizedF32::new(spec.opacity.get().clamp(0.0, 1.0) as f32)
        .unwrap_or(NormalizedF32::ONE);
    Annotation::new_text(text, spec.contents.to_string())
        .with_author(spec.author.as_ref().map(|s| s.to_string()))
        .with_subject(spec.subject.as_ref().map(|s| s.to_string()))
        .with_created(date)
        .with_modified(date)
        .with_color(spec.color.as_ref().map(|c| convert_solid(c).0))
        .with_opacity(opacity)
}

fn convert_icon(icon: NoteIcon) -> krilla::annotation::NoteIcon {
    use krilla::annotation::NoteIcon as K;
    match icon {
        NoteIcon::Comment => K::Comment,
        NoteIcon::Note => K::Note,
        NoteIcon::Help => K::Help,
        NoteIcon::Key => K::Key,
        NoteIcon::Insert => K::Insert,
        NoteIcon::Paragraph => K::Paragraph,
        NoteIcon::NewParagraph => K::NewParagraph,
    }
}

/// Merges rectangles that are on the same line and touch or overlap, so that
/// text with varying styles or nested content doesn't produce one region per
/// text run.
fn merge_rects(rects: Vec<kg::Rect>) -> Vec<kg::Rect> {
    const EPS: f32 = 0.01;
    let mut merged: Vec<kg::Rect> = Vec::with_capacity(rects.len());
    for rect in rects {
        // Rects are on the same line if one contains the other's vertical
        // center.
        let same_line = |a: &kg::Rect, b: &kg::Rect| {
            let center = (b.top() + b.bottom()) / 2.0;
            a.top() <= center && center <= a.bottom()
        };
        if let Some(last) = merged.last_mut()
            && (same_line(last, &rect) || same_line(&rect, last))
            && rect.left() <= last.right() + EPS
            && rect.right() >= last.left() - EPS
        {
            *last = kg::Rect::from_ltrb(
                last.left().min(rect.left()),
                last.top().min(rect.top()),
                last.right().max(rect.right()),
                last.bottom().max(rect.bottom()),
            )
            .unwrap();
        } else {
            merged.push(rect);
        }
    }
    merged
}
