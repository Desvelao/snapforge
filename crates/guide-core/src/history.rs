//! The annotation document and its undo/redo history.
//!
//! Every change goes through [`Editor`], which records a [`Command`] holding
//! enough to undo and redo it exactly. The original screenshot is not part of
//! the document and is never touched by any command.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::annotation::{Annotation, AnnotationId, Kind, PixelRect, Point, Style, MAX_ID};

/// Bumped when a change to the saved format would confuse older versions.
pub const DOCUMENT_VERSION: u32 = 1;

/// Everything about the edit except the pixels of the original image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub version: u32,
    /// Back to front: later entries are drawn on top.
    pub annotations: Vec<Annotation>,
    /// Non-destructive crop in original-image pixels; export renders only this area.
    pub crop: Option<PixelRect>,
    #[serde(default)]
    next_id: u64,
    /// Fields this version does not know; kept so saving does not lose them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            version: DOCUMENT_VERSION,
            annotations: Vec::new(),
            crop: None,
            next_id: 1,
            extra: Map::new(),
        }
    }
}

impl Document {
    pub fn find(&self, id: AnnotationId) -> Option<&Annotation> {
        self.annotations.iter().find(|a| a.id == id)
    }

    pub fn index_of(&self, id: AnnotationId) -> Option<usize> {
        self.annotations.iter().position(|a| a.id == id)
    }

    /// Number for the next numbered marker: one more than the highest in use.
    pub fn next_marker_number(&self) -> u32 {
        self.annotations
            .iter()
            .filter_map(|a| match a.kind {
                Kind::Marker { number, .. } => Some(number),
                _ => None,
            })
            .max()
            .map_or(1, |n| n.saturating_add(1))
    }

    /// Checks a document that came from disk against the image it belongs to.
    pub fn validate(&self, image: (u32, u32)) -> Result<(), String> {
        if self.version > DOCUMENT_VERSION {
            return Err(format!(
                "saved by a newer version (annotation format {}, this version reads up to {DOCUMENT_VERSION})",
                self.version
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for a in &self.annotations {
            a.validate()?;
            if a.id.0 >= MAX_ID {
                return Err(format!("annotation id {} is too large", a.id.0));
            }
            if !seen.insert(a.id) {
                return Err(format!("annotation id {} appears twice", a.id.0));
            }
        }
        if let Some(c) = self.crop {
            let inside = c.w > 0
                && c.h > 0
                && c.x.checked_add(c.w).is_some_and(|r| r <= image.0)
                && c.y.checked_add(c.h).is_some_and(|b| b <= image.1);
            if !inside {
                return Err(format!(
                    "crop {}x{} at ({}, {}) does not fit the {}x{} image",
                    c.w, c.h, c.x, c.y, image.0, image.1
                ));
            }
        }
        Ok(())
    }

    /// Makes sure new ids cannot collide with loaded ones.
    fn repair_next_id(&mut self) {
        let max = self.annotations.iter().map(|a| a.id.0).max().unwrap_or(0);
        self.next_id = self.next_id.max(max.saturating_add(1)).clamp(1, MAX_ID);
    }
}

/// One undoable change. Holds both sides so it can run in either direction.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Add {
        index: usize,
        annotation: Annotation,
    },
    Remove {
        index: usize,
        annotation: Annotation,
    },
    /// Same id before and after: edits, moves, resizes, restyling.
    Replace {
        before: Annotation,
        after: Annotation,
    },
    SetCrop {
        before: Option<PixelRect>,
        after: Option<PixelRect>,
    },
}

impl Command {
    fn apply(&self, doc: &mut Document) {
        match self {
            Command::Add { index, annotation } => {
                let at = (*index).min(doc.annotations.len());
                doc.annotations.insert(at, annotation.clone());
            }
            Command::Remove { annotation, .. } => doc.annotations.retain(|a| a.id != annotation.id),
            Command::Replace { after, .. } => replace_in(doc, after),
            Command::SetCrop { after, .. } => doc.crop = *after,
        }
    }

    fn revert(&self, doc: &mut Document) {
        match self {
            Command::Add { annotation, .. } => doc.annotations.retain(|a| a.id != annotation.id),
            Command::Remove { index, annotation } => {
                let at = (*index).min(doc.annotations.len());
                doc.annotations.insert(at, annotation.clone());
            }
            Command::Replace { before, .. } => replace_in(doc, before),
            Command::SetCrop { before, .. } => doc.crop = *before,
        }
    }
}

fn replace_in(doc: &mut Document, with: &Annotation) {
    if let Some(slot) = doc.annotations.iter_mut().find(|a| a.id == with.id) {
        *slot = with.clone();
    }
}

/// How many changes can be undone before the oldest are forgotten.
pub const DEFAULT_UNDO_DEPTH: usize = 500;

/// A document plus its undo and redo stacks.
#[derive(Debug)]
pub struct Editor {
    doc: Document,
    undo: Vec<Command>,
    redo: Vec<Command>,
    max_undo: usize,
}

impl Editor {
    pub fn new(mut doc: Document) -> Self {
        doc.repair_next_id();
        Self {
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            max_undo: DEFAULT_UNDO_DEPTH,
        }
    }

    /// Keeps at most `depth` (at least 1) undo steps; older ones are forgotten, never the
    /// document's content. Every step holds copies of annotations, so a long session
    /// would otherwise grow without limit.
    pub fn set_undo_depth(&mut self, depth: usize) {
        self.max_undo = depth.max(1);
        self.trim_undo();
    }

    fn trim_undo(&mut self) {
        let extra = self.undo.len().saturating_sub(self.max_undo);
        self.undo.drain(..extra);
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    fn run(&mut self, cmd: Command) {
        cmd.apply(&mut self.doc);
        self.undo.push(cmd);
        self.trim_undo();
        self.redo.clear();
    }

    fn alloc_id(&mut self) -> AnnotationId {
        let id = AnnotationId(self.doc.next_id);
        self.doc.next_id = self.doc.next_id.saturating_add(1);
        id
    }

    /// Adds on top of everything else and returns its new id.
    pub fn add(&mut self, kind: Kind, style: Style) -> AnnotationId {
        let id = self.alloc_id();
        let index = self.doc.annotations.len();
        self.run(Command::Add {
            index,
            annotation: Annotation { id, kind, style },
        });
        id
    }

    /// Adds a numbered marker with the next free number.
    pub fn add_marker(&mut self, center: Point, radius: f32, style: Style) -> AnnotationId {
        let number = self.doc.next_marker_number();
        self.add(
            Kind::Marker {
                center,
                number,
                radius,
            },
            style,
        )
    }

    pub fn remove(&mut self, id: AnnotationId) -> bool {
        let Some(index) = self.doc.index_of(id) else {
            return false;
        };
        let annotation = self.doc.annotations[index].clone();
        self.run(Command::Remove { index, annotation });
        true
    }

    /// Replaces the annotation with the same id. `false` if it does not
    /// exist or nothing would change (so no empty undo step is recorded).
    pub fn replace(&mut self, after: Annotation) -> bool {
        let Some(before) = self.doc.find(after.id).cloned() else {
            return false;
        };
        if before == after {
            return false;
        }
        self.run(Command::Replace { before, after });
        true
    }

    /// Like [`Editor::replace`], but if the latest undo step is also a replace of the
    /// same annotation the two merge into one step (undo goes back to the state before
    /// the first). Used while a slider or colour picker is being dragged.
    pub fn replace_continuing(&mut self, after: Annotation) -> bool {
        let merge = matches!(
            self.undo.last(),
            Some(Command::Replace { after: prev, .. }) if prev.id == after.id
        );
        if !merge {
            return self.replace(after);
        }
        if self.doc.find(after.id) == Some(&after) {
            return false;
        }
        let mut no_change = false;
        if let Some(Command::Replace {
            before,
            after: slot,
            ..
        }) = self.undo.last_mut()
        {
            *slot = after.clone();
            no_change = *before == after;
        }
        replace_in(&mut self.doc, &after);
        if no_change {
            // Dragged back to where it started: nothing left to undo.
            self.undo.pop();
        }
        self.redo.clear();
        true
    }

    pub fn set_crop(&mut self, crop: Option<PixelRect>) -> bool {
        if self.doc.crop == crop {
            return false;
        }
        self.run(Command::SetCrop {
            before: self.doc.crop,
            after: crop,
        });
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(cmd) = self.undo.pop() else {
            return false;
        };
        cmd.revert(&mut self.doc);
        self.redo.push(cmd);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(cmd) = self.redo.pop() else {
            return false;
        };
        cmd.apply(&mut self.doc);
        self.undo.push(cmd);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::{RectF, Rgba};

    fn rect_kind(x: f32) -> Kind {
        Kind::Rectangle {
            rect: RectF {
                x,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        }
    }

    /// Compares what the user sees. The id counter is deliberately excluded: it never
    /// goes backwards, so undone ids are not handed out again.
    fn assert_same(a: &Document, b: &Document) {
        assert!(a.annotations == b.annotations, "annotations differ");
        assert!(
            a.crop == b.crop,
            "crop differs: {:?} vs {:?}",
            a.crop,
            b.crop
        );
    }

    fn ids(e: &Editor) -> Vec<u64> {
        e.document().annotations.iter().map(|a| a.id.0).collect()
    }

    #[test]
    fn add_undo_redo_restores_exact_states() {
        let mut e = Editor::new(Document::default());
        let empty = e.document().clone();
        let id = e.add(rect_kind(1.0), Style::default());
        let one = e.document().clone();
        assert_eq!(ids(&e), vec![id.0]);
        assert!(e.undo());
        assert_same(e.document(), &empty);
        assert!(e.redo());
        assert_same(e.document(), &one);
        assert!(!e.redo());
    }

    #[test]
    fn remove_undo_puts_it_back_at_the_same_z_position() {
        let mut e = Editor::new(Document::default());
        let a = e.add(rect_kind(1.0), Style::default());
        let b = e.add(rect_kind(2.0), Style::default());
        let c = e.add(rect_kind(3.0), Style::default());
        let before = e.document().clone();
        assert!(e.remove(b));
        assert_eq!(ids(&e), vec![a.0, c.0]);
        assert!(e.undo());
        assert_same(e.document(), &before);
        assert!(!e.remove(AnnotationId(999)));
    }

    #[test]
    fn replace_is_undoable_and_skips_no_ops() {
        let mut e = Editor::new(Document::default());
        let id = e.add(rect_kind(1.0), Style::default());
        let original = e.document().find(id).unwrap().clone();
        assert!(
            !e.replace(original.clone()),
            "unchanged replace must not add an undo step"
        );
        let mut moved = original.clone();
        moved.translate(50.0, 0.0);
        assert!(e.replace(moved.clone()));
        assert_eq!(e.document().find(id), Some(&moved));
        assert!(e.undo());
        assert_eq!(e.document().find(id), Some(&original));
        let mut ghost = original;
        ghost.id = AnnotationId(777);
        assert!(!e.replace(ghost));
    }

    #[test]
    fn continuing_replaces_merge_into_one_undo_step() {
        let mut e = Editor::new(Document::default());
        let id = e.add(rect_kind(1.0), Style::default());
        let original = e.document().find(id).unwrap().clone();
        let mut a = original.clone();
        for w in [5.0, 6.0, 7.0, 8.0] {
            a.style.width = w;
            e.replace_continuing(a.clone());
        }
        assert_eq!(e.document().find(id).unwrap().style.width, 8.0);
        assert!(e.undo(), "one undo undoes the whole slider drag");
        assert_eq!(e.document().find(id), Some(&original));
        assert!(e.undo(), "next undo removes the annotation");
        assert!(e.document().annotations.is_empty());
        assert!(e.redo() && e.redo());
        assert_eq!(e.document().find(id).unwrap().style.width, 8.0);
    }

    #[test]
    fn continuing_does_not_merge_across_different_annotations_or_other_edits() {
        let mut e = Editor::new(Document::default());
        let a = e.add(rect_kind(1.0), Style::default());
        let b = e.add(rect_kind(2.0), Style::default());
        let mut ea = e.document().find(a).unwrap().clone();
        ea.style.width = 9.0;
        e.replace_continuing(ea);
        let mut eb = e.document().find(b).unwrap().clone();
        eb.style.width = 11.0;
        e.replace_continuing(eb);
        assert!(e.undo());
        assert_eq!(e.document().find(b).unwrap().style.width, 4.0);
        assert_eq!(
            e.document().find(a).unwrap().style.width,
            9.0,
            "a's change is a separate step"
        );
        // An unrelated edit in between ends the merge.
        e.set_crop(Some(PixelRect {
            x: 0,
            y: 0,
            w: 5,
            h: 5,
        }));
        let mut ea2 = e.document().find(a).unwrap().clone();
        ea2.style.width = 12.0;
        e.replace_continuing(ea2);
        assert!(e.undo());
        assert_eq!(
            e.document().crop,
            Some(PixelRect {
                x: 0,
                y: 0,
                w: 5,
                h: 5
            })
        );
    }

    #[test]
    fn continuing_with_no_change_records_nothing() {
        let mut e = Editor::new(Document::default());
        let id = e.add(rect_kind(1.0), Style::default());
        let same = e.document().find(id).unwrap().clone();
        assert!(!e.replace_continuing(same));
        let mut changed = e.document().find(id).unwrap().clone();
        changed.style.width = 9.0;
        assert!(e.replace_continuing(changed.clone()));
        assert!(!e.replace_continuing(changed));
    }

    #[test]
    fn crop_is_undoable_and_leaves_annotations_alone() {
        let mut e = Editor::new(Document::default());
        e.add(rect_kind(1.0), Style::default());
        let crop = PixelRect {
            x: 5,
            y: 5,
            w: 20,
            h: 20,
        };
        assert!(e.set_crop(Some(crop)));
        assert!(!e.set_crop(Some(crop)));
        assert_eq!(e.document().crop, Some(crop));
        assert!(e.undo());
        assert_eq!(e.document().crop, None);
        assert_eq!(e.document().annotations.len(), 1);
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut e = Editor::new(Document::default());
        e.add(rect_kind(1.0), Style::default());
        e.undo();
        assert!(e.can_redo());
        e.add(rect_kind(2.0), Style::default());
        assert!(!e.can_redo());
    }

    #[test]
    fn ids_are_stable_and_never_reused() {
        let mut e = Editor::new(Document::default());
        let a = e.add(rect_kind(1.0), Style::default());
        e.undo();
        let b = e.add(rect_kind(2.0), Style::default());
        assert_ne!(
            a, b,
            "an id from an undone add must not be handed out again"
        );
        e.remove(b);
        let c = e.add(rect_kind(3.0), Style::default());
        assert!(c.0 > b.0);
        // Redo of an add keeps the original id.
        let mut e2 = Editor::new(Document::default());
        let x = e2.add(rect_kind(1.0), Style::default());
        e2.undo();
        e2.redo();
        assert_eq!(ids(&e2), vec![x.0]);
    }

    #[test]
    fn marker_numbers_continue_from_the_highest() {
        let mut e = Editor::new(Document::default());
        let m1 = e.add_marker(Point::new(0.0, 0.0), 12.0, Style::default());
        let _m2 = e.add_marker(Point::new(5.0, 0.0), 12.0, Style::default());
        e.remove(m1);
        let m3 = e.add_marker(Point::new(9.0, 0.0), 12.0, Style::default());
        let n = |id| match e.document().find(id).unwrap().kind {
            Kind::Marker { number, .. } => number,
            _ => unreachable!(),
        };
        assert_eq!(n(m3), 3);
    }

    #[test]
    fn loaded_documents_cannot_cause_id_collisions() {
        let mut doc = Document::default();
        doc.annotations.push(Annotation {
            id: AnnotationId(41),
            kind: rect_kind(0.0),
            style: Style::default(),
        });
        // A hand-edited file may lack or lower `next_id`.
        let mut e = Editor::new(Document { next_id: 3, ..doc });
        let id = e.add(rect_kind(1.0), Style::default());
        assert_eq!(id, AnnotationId(42));
    }

    #[test]
    fn validate_checks_version_ids_and_crop() {
        let ok = Document::default();
        assert!(ok.validate((100, 100)).is_ok());
        assert!(Document {
            version: 99,
            ..ok.clone()
        }
        .validate((1, 1))
        .unwrap_err()
        .contains("newer version"));
        let dup = Document {
            annotations: vec![
                Annotation {
                    id: AnnotationId(1),
                    kind: rect_kind(0.0),
                    style: Style::default(),
                },
                Annotation {
                    id: AnnotationId(1),
                    kind: rect_kind(1.0),
                    style: Style::default(),
                },
            ],
            ..ok.clone()
        };
        assert!(dup.validate((10, 10)).unwrap_err().contains("twice"));
        let crop = |x, y, w, h| Document {
            crop: Some(PixelRect { x, y, w, h }),
            ..ok.clone()
        };
        assert!(crop(0, 0, 100, 100).validate((100, 100)).is_ok());
        assert!(crop(1, 0, 100, 100).validate((100, 100)).is_err());
        assert!(crop(0, 0, 0, 10).validate((100, 100)).is_err());
        assert!(crop(u32::MAX, 0, 5, 5).validate((100, 100)).is_err());
    }

    #[test]
    fn json_round_trip_keeps_everything_including_next_id() {
        let mut e = Editor::new(Document::default());
        e.add(
            rect_kind(1.0),
            Style {
                fill: Some(Rgba::WHITE),
                ..Style::default()
            },
        );
        e.add_marker(Point::new(3.0, 4.0), 12.0, Style::default());
        e.set_crop(Some(PixelRect {
            x: 1,
            y: 2,
            w: 3,
            h: 4,
        }));
        let json = serde_json::to_string(e.document()).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, e.document());
    }

    /// Tiny deterministic generator so the randomised test is repeatable.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    #[test]
    fn random_edits_undo_all_the_way_back_and_redo_all_the_way_forward() {
        let mut e = Editor::new(Document::default());
        let initial = e.document().clone();
        let mut rng = Lcg(42);
        let mut states = vec![initial.clone()];
        for step in 0..300u32 {
            let doc_ids: Vec<_> = e.document().annotations.iter().map(|a| a.id).collect();
            let changed = match rng.next(5) {
                0 | 1 => {
                    e.add(rect_kind(step as f32), Style::default());
                    true
                }
                2 if !doc_ids.is_empty() => {
                    e.remove(doc_ids[rng.next(doc_ids.len() as u64) as usize])
                }
                3 if !doc_ids.is_empty() => {
                    let mut a = e
                        .document()
                        .find(doc_ids[rng.next(doc_ids.len() as u64) as usize])
                        .unwrap()
                        .clone();
                    a.translate(step as f32 + 1.0, 0.5);
                    e.replace(a)
                }
                4 => e.set_crop(Some(PixelRect {
                    x: step,
                    y: 0,
                    w: 5 + step % 7,
                    h: 5,
                })),
                _ => false,
            };
            if changed {
                states.push(e.document().clone());
            }
        }
        assert!(states.len() > 100, "the run should exercise many edits");
        for expected in states.iter().rev().skip(1) {
            assert!(e.undo());
            assert_same(e.document(), expected);
        }
        assert_same(e.document(), &initial);
        assert!(!e.undo());
        for expected in states.iter().skip(1) {
            assert!(e.redo());
            assert_same(e.document(), expected);
        }
    }

    #[test]
    fn only_the_newest_undo_steps_are_kept_and_the_document_is_untouched() {
        let mut e = Editor::new(Document::default());
        e.set_undo_depth(3);
        for i in 0..10 {
            e.add(rect_kind(1.0 + i as f32), Style::default());
        }
        assert_eq!(
            e.document().annotations.len(),
            10,
            "content is never dropped"
        );
        let mut undone = 0;
        while e.undo() {
            undone += 1;
        }
        assert_eq!(undone, 3);
        assert_eq!(e.document().annotations.len(), 7);
        e.set_undo_depth(0); // at least one step is always kept
        assert!(!e.can_undo() || e.redo());
    }

    #[test]
    fn lowering_the_depth_forgets_the_oldest_steps_at_once() {
        let mut e = Editor::new(Document::default());
        for i in 0..5 {
            e.add(rect_kind(1.0 + i as f32), Style::default());
        }
        e.set_undo_depth(2);
        let mut undone = 0;
        while e.undo() {
            undone += 1;
        }
        assert_eq!(undone, 2);
    }

    #[test]
    fn dragging_a_value_back_to_where_it_started_leaves_no_undo_step() {
        let mut e = Editor::new(Document::default());
        let id = e.add(rect_kind(5.0), Style::default());
        let original = e.document().find(id).unwrap().clone();
        let with_width = |w: f32| Annotation {
            style: Style {
                width: w,
                ..original.style
            },
            ..original.clone()
        };
        assert!(e.replace_continuing(with_width(9.0)));
        assert!(e.replace_continuing(with_width(12.0)));
        assert!(
            e.replace_continuing(original.clone()),
            "the document did change"
        );
        assert_eq!(e.document().find(id), Some(&original));
        // Only the add is left to undo, not an empty change.
        assert!(e.undo());
        assert!(e.document().annotations.is_empty());
        assert!(!e.can_undo());
    }

    #[test]
    fn a_huge_annotation_id_is_refused_and_does_not_overflow_the_counter() {
        let mut e = Editor::new(Document::default());
        let id = e.add(
            Kind::Line {
                from: Point { x: 0.0, y: 0.0 },
                to: Point { x: 5.0, y: 5.0 },
            },
            Style::default(),
        );
        let mut doc = e.document().clone();
        doc.annotations[0].id = AnnotationId(u64::MAX);
        assert!(doc.validate((100, 100)).is_err());
        let mut e = Editor::new(doc); // repairing used to compute u64::MAX + 1
        let fresh = e.add(
            Kind::Line {
                from: Point { x: 0.0, y: 0.0 },
                to: Point { x: 1.0, y: 1.0 },
            },
            Style::default(),
        );
        assert!(fresh != id && fresh.0 < u64::MAX);
    }
}
