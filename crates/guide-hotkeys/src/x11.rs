use std::{
    collections::HashMap,
    sync::{
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use guide_core::{Action, Shortcut};
use x11rb::{
    connection::Connection,
    errors::ReplyError,
    protocol::{
        xproto::{ConnectionExt as _, GrabMode, Mapping, ModMask, Window},
        ErrorKind, Event,
    },
    rust_connection::RustConnection,
};

use crate::{keysym_for, HotkeyBackend, HotkeyError};

// Modifier bits as defined by the X11 protocol (ShiftMask, ControlMask, Mod1..Mod4).
const SHIFT: u16 = 1 << 0;
const CTRL: u16 = 1 << 2;
const ALT: u16 = 1 << 3; // Mod1
const SUPER: u16 = 1 << 6; // Mod4
/// CapsLock and NumLock must not stop a shortcut from working, so every
/// shortcut is grabbed once per combination of these.
const CAPS_LOCK: u16 = 1 << 1;
const NUM_LOCK: u16 = 1 << 4; // Mod2 on virtually every layout
const SIGNIFICANT: u16 = CTRL | SHIFT | ALT | SUPER;
/// Autorepeat of a held shortcut must not start several captures.
const DEBOUNCE: Duration = Duration::from_millis(300);

type Bindings = Arc<Mutex<HashMap<(u8, u16), Action>>>;
/// What the user asked for, kept so the grabs can be redone when the keyboard layout changes.
type Wanted = Arc<Mutex<Vec<(Action, Shortcut)>>>;

pub struct X11Hotkeys {
    conn: Arc<RustConnection>,
    root: Window,
    bindings: Bindings,
    wanted: Wanted,
    rx: Receiver<Action>,
}

impl X11Hotkeys {
    pub fn connect(notify: impl Fn() + Send + Sync + 'static) -> Result<Self, HotkeyError> {
        let (conn, screen) = x11rb::connect(None)
            .map_err(|e| HotkeyError::Backend(format!("cannot connect to the X server: {e}")))?;
        let root = conn.setup().roots[screen].root;
        let conn = Arc::new(conn);
        let bindings: Bindings = Arc::default();
        let wanted: Wanted = Arc::default();
        let (tx, rx) = mpsc::channel();

        let (thread_conn, thread_bindings, thread_wanted) =
            (conn.clone(), bindings.clone(), wanted.clone());
        // Lives until the process exits; it only blocks on the X connection.
        thread::spawn(move || {
            let mut last: HashMap<Action, Instant> = HashMap::new();
            while let Ok(event) = thread_conn.wait_for_event() {
                if let Event::MappingNotify(m) = &event {
                    // The layout changed (keyboard group, setxkbmap): the old keycodes are stale.
                    if m.request == Mapping::KEYBOARD {
                        let wanted = thread_wanted.lock().map(|w| w.clone()).unwrap_or_default();
                        let _ = apply(&thread_conn, root, &thread_bindings, &wanted);
                        let _ = thread_conn.flush();
                    }
                    continue;
                }
                let Event::KeyPress(key) = event else {
                    continue;
                };
                let mods = significant(u16::from(key.state));
                let action = thread_bindings
                    .lock()
                    .ok()
                    .and_then(|b| b.get(&(key.detail, mods)).copied());
                let Some(action) = action else { continue };
                if !not_a_repeat(&mut last, action, Instant::now()) {
                    continue;
                }
                if tx.send(action).is_err() {
                    break;
                }
                notify();
            }
        });
        Ok(Self {
            conn,
            root,
            bindings,
            wanted,
            rx,
        })
    }

    fn ungrab_all(&self) {
        ungrab_all(&self.conn, self.root, &self.bindings);
    }
}

/// The modifiers of a key press that identify a shortcut (not CapsLock, NumLock or button state).
fn significant(state: u16) -> u16 {
    state & SIGNIFICANT
}

/// False when `action` already fired less than [`DEBOUNCE`] ago (autorepeat of a held shortcut).
fn not_a_repeat(last: &mut HashMap<Action, Instant>, action: Action, now: Instant) -> bool {
    if last
        .get(&action)
        .is_some_and(|t| now.duration_since(*t) < DEBOUNCE)
    {
        return false;
    }
    last.insert(action, now);
    true
}

fn ungrab_all(conn: &RustConnection, root: Window, bindings: &Bindings) {
    let _ = conn.ungrab_key(0u8, root, ModMask::ANY);
    if let Ok(mut b) = bindings.lock() {
        b.clear();
    }
}

/// Grabs every wanted shortcut for the current layout, replacing the grabs made before.
fn apply(
    conn: &RustConnection,
    root: Window,
    bindings: &Bindings,
    wanted: &[(Action, Shortcut)],
) -> Vec<(Action, Result<(), HotkeyError>)> {
    ungrab_all(conn, root, bindings);
    wanted
        .iter()
        .map(|(action, shortcut)| {
            let result = grab(conn, root, shortcut).map(|key| {
                if let Ok(mut b) = bindings.lock() {
                    b.insert(key, *action);
                }
            });
            (*action, result)
        })
        .collect()
}

/// Keycode that produces `keysym` without Shift in the first layout group that has it
/// (columns 0, 2, ... of the mapping; odd columns are the shifted level).
fn find_keycode(keysyms: &[u32], per: usize, min: u8, keysym: u32) -> Option<u8> {
    if per == 0 {
        return None;
    }
    (0..per).step_by(2).find_map(|column| {
        keysyms
            .chunks_exact(per)
            .position(|key| key[column] == keysym)
            .and_then(|i| u8::try_from(i).ok()?.checked_add(min))
    })
}

/// Keycode whose unshifted keysym is `keysym`, from the current keyboard layout.
fn keycode_for(conn: &RustConnection, keysym: u32) -> Result<Option<u8>, HotkeyError> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = conn
        .get_keyboard_mapping(min, max - min + 1)
        .map_err(|e| HotkeyError::Backend(e.to_string()))?
        .reply()
        .map_err(|e| HotkeyError::Backend(e.to_string()))?;
    Ok(find_keycode(
        &map.keysyms,
        usize::from(map.keysyms_per_keycode),
        min,
        keysym,
    ))
}

fn grab(
    conn: &RustConnection,
    root: Window,
    shortcut: &Shortcut,
) -> Result<(u8, u16), HotkeyError> {
    let missing = || HotkeyError::KeyNotOnKeyboard(shortcut.key.clone());
    let keysym = keysym_for(&shortcut.key).ok_or_else(missing)?;
    let keycode = keycode_for(conn, keysym)?.ok_or_else(missing)?;
    let mods = [
        (shortcut.ctrl, CTRL),
        (shortcut.shift, SHIFT),
        (shortcut.alt, ALT),
        (shortcut.super_, SUPER),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .fold(0u16, |m, (_, bit)| m | bit);

    for lock in [0, CAPS_LOCK, NUM_LOCK, CAPS_LOCK | NUM_LOCK] {
        let result = conn
            .grab_key(
                false,
                root,
                ModMask::from(mods | lock),
                keycode,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .map_err(|e| HotkeyError::Backend(e.to_string()))?
            .check();
        if let Err(e) = result {
            // Undo the lock variants already grabbed for this shortcut.
            for l in [0, CAPS_LOCK, NUM_LOCK, CAPS_LOCK | NUM_LOCK] {
                let _ = conn.ungrab_key(keycode, root, ModMask::from(mods | l));
            }
            return Err(match e {
                ReplyError::X11Error(x) if x.error_kind == ErrorKind::Access => HotkeyError::InUse,
                other => HotkeyError::Backend(other.to_string()),
            });
        }
    }
    Ok((keycode, mods))
}

impl HotkeyBackend for X11Hotkeys {
    fn name(&self) -> &str {
        "X11"
    }

    fn availability(&self) -> Result<(), String> {
        Ok(())
    }

    fn register(
        &mut self,
        bindings: &[(Action, Shortcut)],
    ) -> Vec<(Action, Result<(), HotkeyError>)> {
        if let Ok(mut w) = self.wanted.lock() {
            *w = bindings.to_vec();
        }
        let out = apply(&self.conn, self.root, &self.bindings, bindings);
        let _ = self.conn.flush();
        out
    }

    fn try_recv(&self) -> Option<Action> {
        self.rx.try_recv().ok()
    }
}

impl Drop for X11Hotkeys {
    fn drop(&mut self) {
        self.ungrab_all();
        let _ = self.conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_and_button_state_do_not_change_which_shortcut_a_press_is() {
        let state = CTRL | SHIFT | CAPS_LOCK | NUM_LOCK | (1 << 8);
        assert_eq!(significant(state), CTRL | SHIFT);
        assert_eq!(significant(ALT | SUPER), ALT | SUPER);
    }

    #[test]
    fn a_held_shortcut_fires_once_per_debounce_period() {
        let mut last = HashMap::new();
        let start = Instant::now();
        let action = Action::CaptureRegion;
        assert!(not_a_repeat(&mut last, action, start));
        assert!(!not_a_repeat(&mut last, action, start + DEBOUNCE / 2));
        assert!(not_a_repeat(&mut last, action, start + DEBOUNCE));
    }

    #[test]
    fn debouncing_one_action_does_not_affect_another() {
        let mut last = HashMap::new();
        let start = Instant::now();
        assert!(not_a_repeat(&mut last, Action::CaptureRegion, start));
        assert!(not_a_repeat(&mut last, Action::CaptureWindow, start));
    }

    const A: u32 = 0x61;
    const CYRILLIC_EF: u32 = 0x06c6;

    #[test]
    fn a_key_in_the_first_group_is_found() {
        // keycodes 8, 9: [a, A, ef, EF] and [b, B, 0, 0]
        let map = [A, 0x41, CYRILLIC_EF, 0x06e6, 0x62, 0x42, 0, 0];
        assert_eq!(find_keycode(&map, 4, 8, A), Some(8));
        assert_eq!(find_keycode(&map, 4, 8, 0x62), Some(9));
    }

    #[test]
    fn a_key_only_in_the_second_group_is_found() {
        // Cyrillic first, Latin second: the letters are in column 2.
        let map = [CYRILLIC_EF, 0x06e6, A, 0x41];
        assert_eq!(find_keycode(&map, 4, 38, A), Some(38));
    }

    #[test]
    fn the_first_group_wins_over_a_later_one() {
        let map = [CYRILLIC_EF, 0, A, 0, A, 0, CYRILLIC_EF, 0];
        assert_eq!(find_keycode(&map, 4, 10, A), Some(11));
    }

    #[test]
    fn the_shifted_level_does_not_count() {
        let map = [0x62, A, 0, 0];
        assert_eq!(find_keycode(&map, 4, 8, A), None);
    }

    #[test]
    fn an_empty_or_odd_layout_finds_nothing() {
        assert_eq!(find_keycode(&[], 0, 8, A), None);
        assert_eq!(find_keycode(&[0x62, 0], 2, 8, A), None);
    }
}
