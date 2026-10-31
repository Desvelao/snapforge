use serde::{Deserialize, Serialize};

/// Which kind of capture the user selected in the UI. Saved in settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModeChoice {
    #[default]
    FullScreen,
    Monitor,
    Region,
    Window,
}
