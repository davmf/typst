use std::sync::Arc;

use indexmap::IndexMap;
use krilla::annotation::{Annotation, TextAnnotation};
use krilla::geom as kg;
use krilla::num::NormalizedF32;
use krilla::page::Page;
use rustc_hash::FxBuildHasher;
use typst_library::foundations::Smart;
use typst_library::introspection::Location;
use typst_library::layout::Size;
use typst_library::model::{AnnotationKind, FrameAnnotation, NoteIcon};

use crate::convert::{FrameContext, GlobalContext};
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
        match spec.kind {
            AnnotationKind::Note if !spec.visible => {
                // Invisible notes are tooltips: cover each line of the body.
                if spec.contents.trim().is_empty() {
                    continue;
                }
                for rect in rects {
                    let text = TextAnnotation::new(rect).with_invisible(true);
                    page.add_annotation(build(gc, spec, text));
                }
            }
            AnnotationKind::Note => {
                // The rect is the space reserved for the icon.
                let Some(&rect) = rects.first() else { continue };
                let text = TextAnnotation::new(rect)
                    .with_icon(convert_icon(spec.icon))
                    .with_open(spec.open);
                page.add_annotation(build(gc, spec, text));
            }
        }
    }
}

/// Builds a krilla annotation with the properties shared by all kinds.
fn build(gc: &GlobalContext, spec: &FrameAnnotation, text: TextAnnotation) -> Annotation {
    let modified = match spec.date {
        Smart::Auto => creation_date(gc),
        Smart::Custom(Some(date)) => convert_datetime(date, None),
        Smart::Custom(None) => None,
    };
    let opacity = NormalizedF32::new(spec.opacity.get().clamp(0.0, 1.0) as f32)
        .unwrap_or(NormalizedF32::ONE);
    Annotation::new_text(text, spec.contents.to_string())
        .with_author(spec.author.as_ref().map(|s| s.to_string()))
        .with_subject(spec.subject.as_ref().map(|s| s.to_string()))
        .with_modified(modified)
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
