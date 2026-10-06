use std::sync::Arc;

use ecow::{EcoString, EcoVec};

use crate::foundations::{Cast, Content, Datetime, Smart, StyleChain, elem};
use crate::introspection::Location;
use crate::layout::Ratio;
use crate::visualize::Color;

/// The kind of a PDF annotation.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum AnnotationKind {
    /// A note, shown as an icon or, if invisible, as a tooltip.
    #[default]
    Note,
}

/// The icon of a visible note annotation.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum NoteIcon {
    /// A speech bubble.
    #[default]
    Comment,
    /// A sheet of paper.
    Note,
    /// A help sign.
    Help,
    /// A key.
    Key,
    /// An insertion caret.
    Insert,
    /// A paragraph sign.
    Paragraph,
    /// A new paragraph sign.
    NewParagraph,
}

/// A PDF annotation, resolved from an element and its styles.
///
/// Laid out content that is covered by the annotation receives a
/// [`FrameItem::Annotation`](crate::layout::FrameItem::Annotation) through the
/// frame modifiers. Exporters group the regions by `location`.
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct FrameAnnotation {
    /// The location of the element that produced the annotation.
    pub location: Location,
    /// The kind of annotation.
    pub kind: AnnotationKind,
    /// The text of the annotation.
    pub contents: EcoString,
    /// The author of the annotation.
    pub author: Option<EcoString>,
    /// A short description of the annotation's subject.
    pub subject: Option<EcoString>,
    /// The color of the annotation.
    pub color: Option<Color>,
    /// The opacity of the annotation.
    pub opacity: Ratio,
    /// The icon of a visible note.
    pub icon: NoteIcon,
    /// Whether the annotation is initially shown open.
    pub open: bool,
    /// Whether a note is visible. Invisible notes act as tooltips.
    pub visible: bool,
    /// The date at which the annotation was last modified.
    pub date: Smart<Option<Datetime>>,
}

impl FrameAnnotation {
    /// An invisible note that shows `text` as a tooltip.
    pub fn tooltip(location: Location, text: EcoString) -> Self {
        Self {
            location,
            kind: AnnotationKind::Note,
            contents: text,
            author: None,
            subject: None,
            color: None,
            opacity: Ratio::one(),
            icon: NoteIcon::Comment,
            open: false,
            visible: false,
            date: Smart::Custom(None),
        }
    }
}

/// Carries the annotations that apply to content through the styles.
///
/// This element is never constructed. It only holds the ghost style that the
/// frame modifiers read.
#[elem]
pub struct AnnotationMarker {
    /// The annotations that should be applied to frames, outermost first.
    #[internal]
    #[ghost]
    pub current: EcoVec<Arc<FrameAnnotation>>,
}

impl AnnotationMarker {
    /// Applies an annotation to `body`, in addition to the annotations that
    /// already apply to it.
    pub fn apply(
        body: Content,
        styles: StyleChain,
        annotation: FrameAnnotation,
    ) -> Content {
        let mut current = styles.get_cloned(Self::current);
        current.push(Arc::new(annotation));
        body.set(Self::current, current)
    }
}
