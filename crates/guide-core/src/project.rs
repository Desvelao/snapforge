//! A project: an ordered list of screenshots (items) with titles, descriptions, captions and
//! editable annotations. A project is either plain or a guide; guide-only settings live in
//! [`GuideSettings`]. Pure data and operations, no I/O.
//!
//! Reordering, duplicating and removing items never change the id of any other item, and image
//! references are plain relative paths that are never rewritten by those operations.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt,
    hash::{BuildHasher, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    annotation::MAX_ID,
    guide::{CompositionSettings, StepLayout},
    history::Document,
};

/// Bumped when a change to `project.json` would confuse older versions.
pub const PROJECT_SCHEMA_VERSION: u32 = 1;

/// Identity of an item inside one project. Never reused, even after deletion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemId(pub u64);

fn default_true() -> bool {
    true
}

/// Settings only a guide has: the text around the items and how the combined image is laid out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuideSettings {
    pub introduction: String,
    /// Optional text shown after the last step.
    pub final_notes: String,
    pub composition: CompositionSettings,
    /// Per-step overrides of the composition settings, by item id.
    #[serde(with = "id_keyed")]
    pub step_layouts: BTreeMap<ItemId, StepLayout>,
    /// Fields this version does not know; kept so saving does not lose them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// JSON object keys are strings, and a map nested in an internally tagged enum is buffered
/// before it is read, which loses the number a key was written from. Read the keys as text.
mod id_keyed {
    use super::{ItemId, StepLayout};
    use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<ItemId, StepLayout>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter()
            .map(|(k, v)| (k.0.to_string(), v))
            .collect::<BTreeMap<_, _>>()
            .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<ItemId, StepLayout>, D::Error> {
        BTreeMap::<String, StepLayout>::deserialize(d)?
            .into_iter()
            .map(|(k, v)| {
                k.parse::<u64>()
                    .map(|n| (ItemId(n), v))
                    .map_err(|_| D::Error::custom(format!("\"{k}\" is not an item id")))
            })
            .collect()
    }
}

/// The subtype of a project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProjectKind {
    /// A set of screenshots to edit and export one by one.
    Plain,
    /// Numbered steps that can also be composed into one long image.
    Guide(GuideSettings),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub caption: String,
    /// Original screenshot, relative to the project folder: `assets/<name>`.
    pub image: String,
    #[serde(default)]
    pub annotations: Document,
    /// Items with `false` stay in the project but are left out of exports.
    #[serde(default = "default_true")]
    pub included: bool,
    /// Fields this version does not know; kept so saving does not lose them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What the caller supplies for a new item; the project assigns the id.
#[derive(Clone, Debug, Default)]
pub struct NewItem {
    pub title: String,
    pub description: String,
    pub caption: String,
    pub image: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectError {
    NewerVersion(u32),
    DuplicateItemId(ItemId),
    ItemIdTooLarge(ItemId),
    BadAssetPath {
        item: ItemId,
        path: String,
        reason: &'static str,
    },
    EmptyProjectId,
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::NewerVersion(v) => write!(
                f,
                "this project was saved by a newer version (format {v}, this version reads up to {PROJECT_SCHEMA_VERSION})"
            ),
            ProjectError::DuplicateItemId(id) => {
                write!(f, "item id {} appears more than once", id.0)
            }
            ProjectError::ItemIdTooLarge(id) => write!(f, "item id {} is too large", id.0),
            ProjectError::BadAssetPath { item, path, reason } => {
                write!(f, "item {} refers to an unsafe image path \"{path}\": {reason}", item.0)
            }
            ProjectError::EmptyProjectId => write!(f, "the project has no project id"),
        }
    }
}

impl std::error::Error for ProjectError {}

/// Why `path` may not be used as an item image, or `None` if it is safe. Only
/// `assets/<name>` is allowed, where `<name>` uses letters, digits, `.`, `_` and
/// `-` and does not start with a dot. That rules out absolute paths, `..`,
/// nested folders, backslashes and anything else that could point outside the project.
pub fn asset_path_problem(path: &str) -> Option<&'static str> {
    let Some(name) = path.strip_prefix("assets/") else {
        return Some("must be inside the assets folder (assets/<name>)");
    };
    if name.is_empty() {
        return Some("has no file name");
    }
    if name.starts_with('.') {
        return Some("file name must not start with a dot");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Some("file name may only use letters, digits, '.', '_' and '-'");
    }
    None
}

/// A safe file or folder name made from free text: lower-case ASCII letters and digits with
/// single dashes between words, at most `max_len` characters, never starting or ending with a
/// dash and never empty (`fallback` is used instead). Characters outside ASCII are dropped, so
/// the result is valid on every file system and cannot contain a path separator or `..`.
pub fn slug(text: &str, max_len: usize, fallback: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(max_len).collect();
    let out = out.trim_matches('-');
    if out.is_empty() {
        fallback.to_string()
    } else {
        out.to_string()
    }
}

/// A fresh random-looking project id (`p-` and 32 hex digits). Not a secret:
/// it only has to differ between projects. Uses the per-process random hash
/// seed and the clock, so no extra dependency is needed.
pub fn new_project_id() -> String {
    let state = std::collections::hash_map::RandomState::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let part = |salt: u64| {
        let mut h = state.build_hasher();
        h.write_u128(nanos);
        h.write_u64(salt);
        h.write_u32(std::process::id());
        h.finish()
    };
    format!("p-{:016x}{:016x}", part(1), part(2))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub project_id: String,
    pub name: String,
    pub kind: ProjectKind,
    pub items: Vec<Item>,
    #[serde(default)]
    next_item_id: u64,
    /// Fields this version does not know; kept so saving does not lose them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Project {
    pub fn new(name: impl Into<String>, kind: ProjectKind) -> Self {
        Self {
            schema_version: PROJECT_SCHEMA_VERSION,
            project_id: new_project_id(),
            name: name.into(),
            kind,
            items: Vec::new(),
            next_item_id: 1,
            extra: Map::new(),
        }
    }

    pub fn new_plain(name: impl Into<String>) -> Self {
        Self::new(name, ProjectKind::Plain)
    }

    pub fn new_guide(name: impl Into<String>) -> Self {
        Self::new(name, ProjectKind::Guide(GuideSettings::default()))
    }

    /// The guide-only settings, or `None` for a plain project.
    pub fn guide(&self) -> Option<&GuideSettings> {
        match &self.kind {
            ProjectKind::Guide(g) => Some(g),
            ProjectKind::Plain => None,
        }
    }

    pub fn guide_mut(&mut self) -> Option<&mut GuideSettings> {
        match &mut self.kind {
            ProjectKind::Guide(g) => Some(g),
            ProjectKind::Plain => None,
        }
    }

    pub fn is_guide(&self) -> bool {
        self.guide().is_some()
    }

    pub fn index_of(&self, id: ItemId) -> Option<usize> {
        self.items.iter().position(|s| s.id == id)
    }

    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.items.iter().find(|s| s.id == id)
    }

    pub fn item_mut(&mut self, id: ItemId) -> Option<&mut Item> {
        self.items.iter_mut().find(|s| s.id == id)
    }

    fn alloc_id(&mut self) -> ItemId {
        let id = ItemId(self.next_item_id.max(1));
        self.next_item_id = id.0.saturating_add(1);
        id
    }

    /// Inserts a new item so it ends up at `index` (clamped to the end) and returns its id.
    pub fn add_item(&mut self, index: usize, new: NewItem) -> ItemId {
        let id = self.alloc_id();
        let at = index.min(self.items.len());
        self.items.insert(
            at,
            Item {
                id,
                title: new.title,
                description: new.description,
                caption: new.caption,
                image: new.image,
                annotations: Document::default(),
                included: true,
                extra: Map::new(),
            },
        );
        id
    }

    pub fn remove_item(&mut self, id: ItemId) -> Option<Item> {
        let i = self.index_of(id)?;
        if let Some(g) = self.guide_mut() {
            g.step_layouts.remove(&id);
        }
        Some(self.items.remove(i))
    }

    /// Moves an item so it ends up at `to` (clamped). Only the order changes.
    pub fn move_item(&mut self, id: ItemId, to: usize) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        let item = self.items.remove(from);
        self.items.insert(to.min(self.items.len()), item);
        true
    }

    /// Copies an item right after itself. The copy has its own id and its own
    /// annotations but points at the same original image: images are shared,
    /// never duplicated on disk. `copy_title` makes the copy's title from the original's, so the
    /// caller can word it in the user's language.
    pub fn duplicate_item(
        &mut self,
        id: ItemId,
        copy_title: impl FnOnce(&str) -> String,
    ) -> Option<ItemId> {
        let i = self.index_of(id)?;
        let new_id = self.alloc_id();
        let mut copy = self.items[i].clone();
        copy.id = new_id;
        copy.title = copy_title(&copy.title);
        self.items.insert(i + 1, copy);
        if let Some(g) = self.guide_mut() {
            if let Some(layout) = g.step_layouts.get(&id).cloned() {
                g.step_layouts.insert(new_id, layout);
            }
        }
        Some(new_id)
    }

    pub fn set_included(&mut self, id: ItemId, included: bool) -> bool {
        self.item_mut(id).map(|s| s.included = included).is_some()
    }

    /// Items that will be exported, in order.
    pub fn included_items(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|s| s.included)
    }

    /// The per-step layout override of a guide item, if it has one.
    pub fn step_layout(&self, id: ItemId) -> Option<&StepLayout> {
        self.guide()?.step_layouts.get(&id)
    }

    /// Every image path some item refers to.
    pub fn referenced_assets(&self) -> BTreeSet<&str> {
        self.items.iter().map(|s| s.image.as_str()).collect()
    }

    /// Structural checks for a project that came from disk.
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.schema_version > PROJECT_SCHEMA_VERSION {
            return Err(ProjectError::NewerVersion(self.schema_version));
        }
        if self.project_id.trim().is_empty() {
            return Err(ProjectError::EmptyProjectId);
        }
        let mut seen = HashSet::new();
        for s in &self.items {
            if s.id.0 >= MAX_ID {
                return Err(ProjectError::ItemIdTooLarge(s.id));
            }
            if !seen.insert(s.id) {
                return Err(ProjectError::DuplicateItemId(s.id));
            }
            if let Some(reason) = asset_path_problem(&s.image) {
                return Err(ProjectError::BadAssetPath {
                    item: s.id,
                    path: s.image.clone(),
                    reason,
                });
            }
        }
        Ok(())
    }

    /// Brings what a hand-edited or damaged file may hold back to what the layout engine supports:
    /// composition values are clamped, per-step layouts of steps that do not exist are dropped and
    /// per-step widths are clamped like the guide-wide one. One sentence per change.
    pub fn sanitize(&mut self) -> Vec<String> {
        let ids: HashSet<ItemId> = self.items.iter().map(|s| s.id).collect();
        let Some(guide) = self.guide_mut() else {
            return Vec::new();
        };
        let mut notes = guide.composition.sanitize();
        guide.step_layouts.retain(|id, _| {
            let known = ids.contains(id);
            if !known {
                notes.push(format!(
                    "dropped the layout of step {}, which does not exist",
                    id.0
                ));
            }
            known
        });
        let (lo, hi) = CompositionSettings::MAX_WIDTH;
        for (id, layout) in &mut guide.step_layouts {
            if let Some(w) = layout.max_width.filter(|w| !(lo..=hi).contains(w)) {
                let fixed = w.clamp(lo, hi);
                notes.push(format!(
                    "step {} max_width {w} is outside {lo}-{hi}; using {fixed}",
                    id.0
                ));
                layout.max_width = Some(fixed);
            }
        }
        notes
    }

    /// Makes sure new item ids cannot collide with loaded ones.
    pub fn repair(&mut self) {
        let max = self.items.iter().map(|s| s.id.0).max().unwrap_or(0);
        self.next_item_id = self
            .next_item_id
            .max(max.saturating_add(1))
            .clamp(1, MAX_ID);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(title: &str, image: &str) -> NewItem {
        NewItem {
            title: title.into(),
            image: image.into(),
            ..NewItem::default()
        }
    }

    fn project_of(n: usize) -> (Project, Vec<ItemId>) {
        let mut g = Project::new_guide("Guide");
        let ids = (0..n)
            .map(|i| {
                g.add_item(
                    usize::MAX,
                    new(
                        &format!("s{i}"),
                        &format!("assets/step-{}-original.png", i + 1),
                    ),
                )
            })
            .collect();
        (g, ids)
    }

    fn order(g: &Project) -> Vec<u64> {
        g.items.iter().map(|s| s.id.0).collect()
    }

    #[test]
    fn slugs_are_portable_file_names() {
        assert_eq!(slug("Open the Settings!", 60, "x"), "open-the-settings");
        assert_eq!(slug("  --Weird!!  Title??  ", 60, "x"), "weird-title");
        assert_eq!(slug("../../etc/passwd", 60, "x"), "etc-passwd");
        assert_eq!(
            slug("a/b\\c:d*e?f\"g<h>i|j", 60, "x"),
            "a-b-c-d-e-f-g-h-i-j"
        );
        assert_eq!(slug("Ünïcode ñame", 60, "x"), "n-code-ame");
        assert_eq!(
            slug("日本語", 60, "step"),
            "step",
            "nothing left: the fallback"
        );
        assert_eq!(slug("", 60, "guide"), "guide");
        assert_eq!(slug(&"abc ".repeat(100), 10, "x"), "abc-abc-ab");
        assert_eq!(
            slug("abcdefghi jklm", 9, "x"),
            "abcdefghi",
            "cutting must not leave a trailing dash"
        );
        assert_eq!(
            slug("abcdefgh ijk", 9, "x"),
            "abcdefgh",
            "cutting must not leave a trailing dash"
        );
        assert!(!slug("CON", 60, "x").contains('.'));
    }

    #[test]
    fn new_projects_have_distinct_well_formed_project_ids() {
        let ids: HashSet<String> = (0..500)
            .map(|_| Project::new_plain("x").project_id)
            .collect();
        assert_eq!(ids.len(), 500);
        assert!(ids.iter().all(|i| i.len() == 34
            && i.starts_with("p-")
            && i[2..].chars().all(|c| c.is_ascii_hexdigit())));
    }

    #[test]
    fn items_get_increasing_ids_and_can_be_inserted_anywhere() {
        let (mut g, ids) = project_of(3);
        assert_eq!(ids, vec![ItemId(1), ItemId(2), ItemId(3)]);
        let mid = g.add_item(1, new("mid", "assets/m.png"));
        assert_eq!(order(&g), vec![1, mid.0, 2, 3]);
        let first = g.add_item(0, new("first", "assets/f.png"));
        assert_eq!(g.items[0].id, first);
        let last = g.add_item(999, new("last", "assets/l.png"));
        assert_eq!(g.items.last().unwrap().id, last);
    }

    #[test]
    fn ids_are_never_reused_after_removal() {
        let (mut g, ids) = project_of(3);
        g.remove_item(ids[2]);
        let again = g.add_item(usize::MAX, new("new", "assets/n.png"));
        assert!(again.0 > ids[2].0);
        assert!(g.remove_item(ItemId(999)).is_none());
    }

    #[test]
    fn moving_changes_only_the_order() {
        let (mut g, ids) = project_of(5);
        let before: Vec<Item> = g.items.clone();
        assert!(g.move_item(ids[0], 3));
        assert_eq!(order(&g), vec![2, 3, 4, 1, 5]);
        assert!(g.move_item(ids[4], 0));
        assert_eq!(order(&g), vec![5, 2, 3, 4, 1]);
        assert!(g.move_item(ids[1], 999), "to is clamped to the end");
        assert_eq!(*order(&g).last().unwrap(), 2);
        assert!(!g.move_item(ItemId(77), 0));
        for s in &before {
            assert_eq!(
                g.item(s.id),
                Some(s),
                "an item's content and image path are untouched by reordering"
            );
        }
    }

    #[test]
    fn every_possible_move_keeps_ids_and_images_intact() {
        let n = 6;
        for from in 0..n {
            for to in 0..n {
                let (mut g, ids) = project_of(n);
                let images: Vec<String> = g.items.iter().map(|s| s.image.clone()).collect();
                g.move_item(ids[from], to);
                assert_eq!(g.items[to].id, ids[from], "moved item lands at {to}");
                let mut sorted = order(&g);
                sorted.sort_unstable();
                assert_eq!(
                    sorted,
                    (1..=n as u64).collect::<Vec<_>>(),
                    "no id lost or duplicated"
                );
                for s in &g.items {
                    assert_eq!(
                        s.image,
                        images[s.id.0 as usize - 1],
                        "image still belongs to its item"
                    );
                }
                g.validate().unwrap();
            }
        }
    }

    #[test]
    fn duplicating_shares_the_image_and_copies_the_annotations() {
        let (mut g, ids) = project_of(2);
        let mut e =
            crate::Editor::new(std::mem::take(&mut g.item_mut(ids[0]).unwrap().annotations));
        e.add(
            crate::Kind::Line {
                from: crate::Point::new(0.0, 0.0),
                to: crate::Point::new(5.0, 5.0),
            },
            crate::Style::default(),
        );
        g.item_mut(ids[0]).unwrap().annotations = e.document().clone();

        let copy = g.duplicate_item(ids[0], |t| format!("{t} (copy)")).unwrap();
        assert_eq!(order(&g), vec![1, copy.0, 2]);
        let (a, b) = (g.item(ids[0]).unwrap(), g.item(copy).unwrap());
        assert_eq!(
            a.image, b.image,
            "the original screenshot is shared, not copied"
        );
        assert_eq!(a.annotations.annotations, b.annotations.annotations);
        assert_eq!(b.title, "s0 (copy)");
        // Editing the copy must not change the original.
        g.item_mut(copy).unwrap().annotations.annotations.clear();
        assert_eq!(g.item(ids[0]).unwrap().annotations.annotations.len(), 1);
        assert!(g.duplicate_item(ItemId(99), str::to_string).is_none());
    }

    #[test]
    fn inclusion_controls_what_is_exported_without_removing_items() {
        let (mut g, ids) = project_of(3);
        assert!(g.set_included(ids[1], false));
        assert_eq!(g.items.len(), 3);
        assert_eq!(
            g.included_items().map(|s| s.id.0).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert!(!g.set_included(ItemId(50), false));
    }

    #[test]
    fn referenced_assets_lists_each_image_once_even_when_shared() {
        let (mut g, ids) = project_of(2);
        g.duplicate_item(ids[0], str::to_string);
        let refs: Vec<&str> = g.referenced_assets().into_iter().collect();
        assert_eq!(
            refs,
            vec!["assets/step-1-original.png", "assets/step-2-original.png"]
        );
    }

    #[test]
    fn safe_and_unsafe_asset_paths() {
        for ok in [
            "assets/step-1-original.png",
            "assets/a.b_c-D9.jpg",
            "assets/x",
        ] {
            assert_eq!(asset_path_problem(ok), None, "{ok}");
        }
        for bad in [
            "",
            "assets/",
            "/etc/passwd",
            "../x.png",
            "assets/../x.png",
            "assets/../../etc/passwd",
            "assets//x.png",
            "assets/a/b.png",
            "assets\\x.png",
            "assets/.hidden",
            "assets/..",
            "assets/x\0.png",
            "C:\\x.png",
            "x.png",
            "assets/with space.png",
            "Assets/x.png",
            "./assets/x.png",
            "assets/é.png",
            "file:///etc/passwd",
        ] {
            assert!(
                asset_path_problem(bad).is_some(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn validate_catches_duplicates_bad_paths_and_newer_versions() {
        let (g, _) = project_of(2);
        assert!(g.validate().is_ok());
        let mut dup = g.clone();
        dup.items[1].id = dup.items[0].id;
        assert_eq!(
            dup.validate(),
            Err(ProjectError::DuplicateItemId(ItemId(1)))
        );
        let mut bad = g.clone();
        bad.items[0].image = "../../etc/passwd".into();
        assert!(matches!(
            bad.validate(),
            Err(ProjectError::BadAssetPath { .. })
        ));
        let mut newer = g.clone();
        newer.schema_version = 99;
        assert_eq!(newer.validate(), Err(ProjectError::NewerVersion(99)));
        assert!(newer
            .validate()
            .unwrap_err()
            .to_string()
            .contains("newer version"));
        let mut noid = g;
        noid.project_id = " ".into();
        assert_eq!(noid.validate(), Err(ProjectError::EmptyProjectId));
    }

    #[test]
    fn loaded_projects_cannot_cause_item_id_collisions() {
        let (g, _) = project_of(3);
        let mut json: serde_json::Value = serde_json::to_value(&g).unwrap();
        json["next_item_id"] = serde_json::json!(1); // hand-edited / missing
        let mut loaded: Project = serde_json::from_value(json).unwrap();
        loaded.repair();
        let id = loaded.add_item(usize::MAX, new("n", "assets/n.png"));
        assert_eq!(id, ItemId(4));
    }

    #[test]
    fn plain_projects_have_no_guide_settings_and_guides_do() {
        let plain = Project::new_plain("P");
        assert!(!plain.is_guide() && plain.guide().is_none());
        let guide = Project::new_guide("G");
        assert!(guide.is_guide());
        assert_eq!(guide.guide().unwrap(), &GuideSettings::default());
    }

    #[test]
    fn removing_and_duplicating_items_keep_step_layouts_in_step() {
        let (mut g, ids) = project_of(2);
        let layout = StepLayout {
            max_width: Some(600),
            show_description: Some(false),
        };
        g.guide_mut()
            .unwrap()
            .step_layouts
            .insert(ids[0], layout.clone());
        let copy = g.duplicate_item(ids[0], |t| format!("{t} (copy)")).unwrap();
        assert_eq!(g.step_layout(copy), Some(&layout));
        g.remove_item(ids[0]);
        assert_eq!(g.step_layout(ids[0]), None);
        assert_eq!(g.step_layout(copy), Some(&layout));
    }

    #[test]
    fn unknown_fields_survive_a_round_trip_at_every_level() {
        let text = r#"{
            "schema_version": 1, "project_id": "p-1", "name": "T", "future_project_field": {"a": [1, 2]},
            "kind": {"type": "guide", "future_guide_field": 5, "composition": {"margin": 10, "future_setting": true}},
            "items": [{"id": 3, "title": "S", "image": "assets/a.png", "future_item_field": "keep me",
                       "annotations": {"version": 1, "annotations": [], "crop": null, "next_id": 1, "future_doc_field": 7}}]
        }"#;
        let g: Project = serde_json::from_str(text).unwrap();
        assert_eq!(
            g.extra["future_project_field"],
            serde_json::json!({"a": [1, 2]})
        );
        assert_eq!(g.items[0].extra["future_item_field"], "keep me");
        assert_eq!(g.items[0].annotations.extra["future_doc_field"], 7);
        let settings = g.guide().unwrap();
        assert_eq!(settings.extra["future_guide_field"], 5);
        assert_eq!(settings.composition.extra["future_setting"], true);
        assert_eq!(
            settings.composition.margin, 10,
            "known settings still read normally"
        );

        let back: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(
            back["future_project_field"],
            serde_json::json!({"a": [1, 2]})
        );
        assert_eq!(back["items"][0]["future_item_field"], "keep me");
        assert_eq!(back["items"][0]["annotations"]["future_doc_field"], 7);
        assert_eq!(back["kind"]["future_guide_field"], 5);
        assert_eq!(back["kind"]["composition"]["future_setting"], true);
        // Known fields are written once, not duplicated into the extras.
        assert!(g.extra.get("name").is_none() && g.items[0].extra.get("title").is_none());
    }

    #[test]
    fn json_round_trip_and_defaults_for_missing_optional_fields() {
        let (mut g, ids) = project_of(2);
        g.guide_mut().unwrap().introduction = "Intro".into();
        g.item_mut(ids[0]).unwrap().description = "Click the button".into();
        g.item_mut(ids[0]).unwrap().caption = "The button".into();
        g.guide_mut().unwrap().step_layouts.insert(
            ids[1],
            StepLayout {
                max_width: Some(600),
                show_description: Some(false),
            },
        );
        let back: Project = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(back, g);

        let plain = Project::new_plain("P");
        let back: Project = serde_json::from_str(&serde_json::to_string(&plain).unwrap()).unwrap();
        assert_eq!(back, plain);

        // A minimal hand-written item only needs id, title and image.
        let minimal = r#"{"schema_version":1,"project_id":"p-1","name":"T","kind":{"type":"plain"},"items":[{"id":7,"title":"A","image":"assets/a.png"}]}"#;
        let m: Project = serde_json::from_str(minimal).unwrap();
        let s = &m.items[0];
        assert!(
            s.included
                && s.description.is_empty()
                && s.caption.is_empty()
                && s.annotations.annotations.is_empty()
        );
        assert!(!m.is_guide());
    }

    #[test]
    fn a_manifest_without_a_kind_is_refused() {
        let old = r#"{"schema_version":1,"project_id":"p-1","title":"T","steps":[]}"#;
        assert!(serde_json::from_str::<Project>(old).is_err());
    }

    #[test]
    fn sanitize_drops_layouts_of_missing_steps_and_clamps_widths() {
        let (mut g, ids) = project_of(2);
        let guide = g.guide_mut().unwrap();
        guide.composition.margin = 99_999;
        guide.step_layouts.insert(
            ids[0],
            StepLayout {
                max_width: Some(0),
                show_description: Some(false),
            },
        );
        guide.step_layouts.insert(
            ids[1],
            StepLayout {
                max_width: Some(1_000_000_000),
                show_description: None,
            },
        );
        guide.step_layouts.insert(ItemId(99), StepLayout::default());

        let notes = g.sanitize();
        assert_eq!(notes.len(), 4, "{notes:?}");
        let guide = g.guide().unwrap();
        assert_eq!(guide.composition.margin, CompositionSettings::MARGIN.1);
        assert_eq!(guide.step_layouts.len(), 2);
        assert_eq!(
            guide.step_layouts[&ids[0]].max_width,
            Some(CompositionSettings::MAX_WIDTH.0)
        );
        assert_eq!(guide.step_layouts[&ids[0]].show_description, Some(false));
        assert_eq!(
            guide.step_layouts[&ids[1]].max_width,
            Some(CompositionSettings::MAX_WIDTH.1)
        );
    }

    #[test]
    fn sanitize_leaves_a_good_project_alone() {
        let (mut g, ids) = project_of(1);
        g.guide_mut().unwrap().step_layouts.insert(
            ids[0],
            StepLayout {
                max_width: Some(800),
                show_description: None,
            },
        );
        let before = g.clone();
        assert!(g.sanitize().is_empty());
        assert_eq!(g, before);
        assert!(Project::new_plain("Plain").sanitize().is_empty());
    }

    #[test]
    fn a_huge_item_id_is_refused_and_repairing_it_cannot_overflow() {
        let (mut g, _) = project_of(1);
        g.items[0].id = ItemId(u64::MAX);
        g.repair(); // would overflow with `max + 1`
        assert_eq!(
            g.validate(),
            Err(ProjectError::ItemIdTooLarge(ItemId(u64::MAX)))
        );
        let (mut g, _) = project_of(1);
        g.next_item_id = u64::MAX;
        g.repair();
        let fresh = g.alloc_id();
        assert!(
            g.items.iter().all(|s| s.id != fresh),
            "a new id never collides"
        );
    }
}
