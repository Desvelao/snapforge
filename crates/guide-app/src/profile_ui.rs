//! The profile chooser shared by the main window and the editor: pick a profile,
//! or create, rename and delete one. It only reports what the user asked for;
//! the caller applies it to the settings.

use eframe::egui;
use guide_storage::{MAX_PROFILES, MAX_PROFILE_NAME};

use crate::i18n::{tr, tr_fmt};

/// What the user asked the profile list to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileRequest {
    Select(String),
    New(String),
    Rename { from: String, to: String },
    Delete(String),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Idle,
    New,
    Rename,
    ConfirmDelete,
}

#[derive(Default)]
pub struct ProfilePicker {
    mode: Mode,
    text: String,
    focus_pending: bool,
}

/// Whether `name` can be used for a new profile, or as the new name of `renaming`.
fn name_is_usable(names: &[String], name: &str, renaming: Option<&str>) -> bool {
    let name = name.trim();
    !name.is_empty()
        && !names
            .iter()
            .any(|n| n.to_lowercase() == name.to_lowercase() && Some(n.as_str()) != renaming)
        && (renaming.is_some() || names.len() < MAX_PROFILES)
}

impl ProfilePicker {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        names: &[String],
        active: &str,
    ) -> Option<ProfileRequest> {
        let mut request = None;
        match self.mode {
            Mode::Idle => {
                ui.label(tr("Profile"));
                egui::ComboBox::from_id_salt(id)
                    .selected_text(active)
                    .show_ui(ui, |ui| {
                        for n in names {
                            if ui.selectable_label(n == active, n).clicked() && n != active {
                                request = Some(ProfileRequest::Select(n.clone()));
                            }
                        }
                    })
                    .response
                    .on_hover_text(tr(
                        "The tool settings used for click capture and the editor",
                    ));
                if ui
                    .add_enabled(names.len() < MAX_PROFILES, egui::Button::new(tr("New")))
                    .on_hover_text(tr("Add a profile with a copy of the current settings"))
                    .clicked()
                {
                    self.mode = Mode::New;
                    self.text.clear();
                    self.focus_pending = true;
                }
                if ui.button(tr("Rename")).clicked() {
                    self.mode = Mode::Rename;
                    self.text = active.to_string();
                    self.focus_pending = true;
                }
                if ui
                    .add_enabled(names.len() > 1, egui::Button::new(tr("Delete")))
                    .clicked()
                {
                    self.mode = Mode::ConfirmDelete;
                }
            }
            Mode::New | Mode::Rename => {
                let renaming = (self.mode == Mode::Rename).then_some(active);
                ui.label(tr("Profile name"));
                let edit = ui.add(
                    crate::theme::text_singleline(&mut self.text).char_limit(MAX_PROFILE_NAME),
                );
                if std::mem::take(&mut self.focus_pending) {
                    edit.request_focus();
                }
                let usable = name_is_usable(names, &self.text, renaming);
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui
                    .add_enabled(usable, egui::Button::new(tr("OK")))
                    .clicked()
                    || enter)
                    && usable
                {
                    let name = self.text.trim().to_string();
                    request = Some(match renaming {
                        Some(from) => ProfileRequest::Rename {
                            from: from.to_string(),
                            to: name,
                        },
                        None => ProfileRequest::New(name),
                    });
                    self.mode = Mode::Idle;
                } else if ui.button(tr("Cancel")).clicked() {
                    self.mode = Mode::Idle;
                }
                if !usable && !self.text.trim().is_empty() {
                    ui.label(tr("That name is already used"));
                }
            }
            Mode::ConfirmDelete => {
                ui.label(tr_fmt(
                    "Delete the profile \"{name}\"?",
                    &[("name", &active)],
                ));
                if ui.button(tr("Delete")).clicked() {
                    request = Some(ProfileRequest::Delete(active.to_string()));
                    self.mode = Mode::Idle;
                } else if ui.button(tr("Cancel")).clicked() {
                    self.mode = Mode::Idle;
                }
            }
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec!["Default".into(), "Docs".into()]
    }

    #[test]
    fn empty_and_taken_names_are_not_usable_but_renaming_to_a_new_case_is() {
        assert!(!name_is_usable(&names(), "  ", None));
        assert!(!name_is_usable(&names(), "docs", None));
        assert!(name_is_usable(&names(), "Manuals", None));
        assert!(!name_is_usable(&names(), "default", Some("Docs")));
        assert!(name_is_usable(&names(), "DOCS", Some("Docs")));
    }

    #[test]
    fn no_more_profiles_can_be_added_at_the_limit() {
        let full: Vec<String> = (0..MAX_PROFILES).map(|i| format!("p{i}")).collect();
        assert!(!name_is_usable(&full, "extra", None));
        assert!(name_is_usable(&full, "renamed", Some("p0")));
    }
}
