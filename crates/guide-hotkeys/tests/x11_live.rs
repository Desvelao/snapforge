//! Live tests against an X server, using XTEST to send fake key presses.
//! Run serially (`-j1`): fake modifier state is global to the X server.
//! Skipped without DISPLAY unless SNAPFORGE_EXPECT_WINDOWS is set.
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread::sleep,
    time::{Duration, Instant},
};

use guide_core::{Action, Shortcut};
use guide_hotkeys::{
    keysym_for, ClickEvent, HotkeyBackend, HotkeyError, PointerBackend, X11Hotkeys, X11Pointer,
};
use x11rb::{
    connection::Connection,
    protocol::{xproto::ConnectionExt as _, xtest::ConnectionExt as _},
    rust_connection::RustConnection,
};

const CONTROL_L: u32 = 0xFFE3;
const SHIFT_L: u32 = 0xFFE1;
const ALT_L: u32 = 0xFFE9;
const NUM_LOCK: u32 = 0xFF7F;

fn display_available() -> bool {
    if std::env::var_os("DISPLAY").is_some() {
        return true;
    }
    assert!(
        std::env::var_os("SNAPFORGE_EXPECT_WINDOWS").is_none(),
        "no DISPLAY"
    );
    false
}

struct Keyboard {
    conn: RustConnection,
    root: u32,
    min: u8,
    per: usize,
    keysyms: Vec<u32>,
}

impl Keyboard {
    fn new() -> Self {
        let (conn, screen) = x11rb::connect(None).unwrap();
        let setup = conn.setup();
        let (root, min, max) = (
            setup.roots[screen].root,
            setup.min_keycode,
            setup.max_keycode,
        );
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .unwrap()
            .reply()
            .unwrap();
        Self {
            conn,
            root,
            min,
            per: map.keysyms_per_keycode.into(),
            keysyms: map.keysyms,
        }
    }

    fn keycode(&self, keysym: u32) -> u8 {
        (self.min..=255)
            .find(|kc| self.keysyms.get(usize::from(kc - self.min) * self.per) == Some(&keysym))
            .unwrap_or_else(|| panic!("keysym {keysym:#x} not in the keymap"))
    }

    fn send(&self, keysym: u32, press: bool) {
        let kind = if press { 2 } else { 3 }; // KeyPress / KeyRelease
        self.conn
            .xtest_fake_input(kind, self.keycode(keysym), 0, self.root, 0, 0, 0)
            .unwrap();
        self.conn.flush().unwrap();
        sleep(Duration::from_millis(30));
    }

    /// Presses the modifiers, then the key, then releases everything.
    fn chord(&self, modifiers: &[u32], key: &str) {
        let ks = keysym_for(key).unwrap();
        for m in modifiers {
            self.send(*m, true);
        }
        self.send(ks, true);
        self.send(ks, false);
        for m in modifiers.iter().rev() {
            self.send(*m, false);
        }
    }
}

fn wait_for(backend: &dyn HotkeyBackend, timeout: Duration) -> Option<Action> {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if let Some(a) = backend.try_recv() {
            return Some(a);
        }
        sleep(Duration::from_millis(20));
    }
    None
}

fn shortcut(s: &str) -> Shortcut {
    Shortcut::parse(s).unwrap()
}

fn backend() -> (X11Hotkeys, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let b = X11Hotkeys::connect(move || {
        c.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    (b, count)
}

#[test]
fn registered_shortcut_delivers_its_action_and_notifies() {
    if !display_available() {
        return;
    }
    let (mut b, notified) = backend();
    let r = b.register(&[(Action::CaptureRegion, shortcut("Ctrl+Shift+R"))]);
    assert_eq!(r, vec![(Action::CaptureRegion, Ok(()))]);
    Keyboard::new().chord(&[CONTROL_L, SHIFT_L], "R");
    assert_eq!(
        wait_for(&b, Duration::from_secs(2)),
        Some(Action::CaptureRegion)
    );
    assert_eq!(notified.load(Ordering::SeqCst), 1);
}

#[test]
fn works_with_numlock_on() {
    if !display_available() {
        return;
    }
    let (mut b, _) = backend();
    b.register(&[(Action::CaptureFullScreen, shortcut("Ctrl+Shift+F"))]);
    let kb = Keyboard::new();
    kb.send(NUM_LOCK, true);
    kb.send(NUM_LOCK, false); // NumLock is now on
    let mask = kb
        .conn
        .query_pointer(kb.root)
        .unwrap()
        .reply()
        .unwrap()
        .mask;
    assert!(
        u16::from(mask) & (1 << 4) != 0,
        "NumLock did not turn on, test would prove nothing"
    );
    kb.chord(&[CONTROL_L, SHIFT_L], "F");
    let got = wait_for(&b, Duration::from_secs(2));
    kb.send(NUM_LOCK, true);
    kb.send(NUM_LOCK, false); // restore
    assert_eq!(got, Some(Action::CaptureFullScreen));
}

#[test]
fn extra_modifier_does_not_trigger() {
    if !display_available() {
        return;
    }
    let (mut b, _) = backend();
    b.register(&[(Action::CaptureWindow, shortcut("Ctrl+Shift+W"))]);
    Keyboard::new().chord(&[CONTROL_L, SHIFT_L, ALT_L], "W");
    assert_eq!(wait_for(&b, Duration::from_millis(500)), None);
}

#[test]
fn same_key_from_a_second_client_reports_in_use() {
    if !display_available() {
        return;
    }
    let (mut first, _) = backend();
    let (mut second, _) = backend();
    let binding = [(Action::CaptureRegion, shortcut("Ctrl+Alt+Q"))];
    assert_eq!(first.register(&binding)[0].1, Ok(()));
    assert_eq!(second.register(&binding)[0].1, Err(HotkeyError::InUse));
    // One failure must not affect other bindings in the same call.
    let r = second.register(&[
        (Action::CaptureRegion, shortcut("Ctrl+Alt+Q")),
        (Action::CaptureWindow, shortcut("Ctrl+Alt+E")),
    ]);
    assert_eq!(r[0].1, Err(HotkeyError::InUse));
    assert_eq!(r[1].1, Ok(()));
}

#[test]
fn reregistering_replaces_the_old_shortcut() {
    if !display_available() {
        return;
    }
    let (mut b, _) = backend();
    b.register(&[(Action::CaptureRegion, shortcut("Ctrl+Shift+T"))]);
    b.register(&[(Action::CaptureRegion, shortcut("Ctrl+Shift+Y"))]);
    let kb = Keyboard::new();
    kb.chord(&[CONTROL_L, SHIFT_L], "T");
    assert_eq!(
        wait_for(&b, Duration::from_millis(500)),
        None,
        "old shortcut still active"
    );
    kb.chord(&[CONTROL_L, SHIFT_L], "Y");
    assert_eq!(
        wait_for(&b, Duration::from_secs(2)),
        Some(Action::CaptureRegion)
    );
}

#[test]
fn dropping_the_backend_releases_the_shortcut() {
    if !display_available() {
        return;
    }
    let binding = [(Action::CaptureRegion, shortcut("Ctrl+Alt+U"))];
    {
        let (mut b, _) = backend();
        assert_eq!(b.register(&binding)[0].1, Ok(()));
    }
    let (mut again, _) = backend();
    assert_eq!(again.register(&binding)[0].1, Ok(()));
}

fn click_at(x: i16, y: i16, button: u8) {
    let (conn, screen) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[screen].root;
    conn.xtest_fake_input(6, 0, 0, root, x, y, 0).unwrap(); // MotionNotify
    conn.xtest_fake_input(4, button, 0, root, 0, 0, 0).unwrap(); // ButtonPress
    conn.xtest_fake_input(5, button, 0, root, 0, 0, 0).unwrap(); // ButtonRelease
    conn.flush().unwrap();
    sleep(Duration::from_millis(30));
}

fn next_click(b: &dyn PointerBackend, timeout: Duration) -> Option<ClickEvent> {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if let Some(c) = b.try_recv() {
            return Some(c);
        }
        sleep(Duration::from_millis(20));
    }
    None
}

#[test]
fn a_mouse_click_anywhere_is_reported_with_its_position_and_scrolling_is_not() {
    if !display_available() {
        return;
    }
    let notified = Arc::new(AtomicUsize::new(0));
    let n = notified.clone();
    let pointer = X11Pointer::connect(move || {
        n.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    click_at(300, 200, 4); // wheel up
    assert_eq!(next_click(&pointer, Duration::from_millis(300)), None);
    click_at(300, 200, 1);
    let click = next_click(&pointer, Duration::from_secs(2)).expect("no click arrived");
    assert_eq!(
        click,
        ClickEvent {
            x: 300,
            y: 200,
            button: 1
        }
    );
    assert!(notified.load(Ordering::SeqCst) >= 1);
    // Not a right click: on the desktop it would open the window manager's menu,
    // which grabs the keyboard and breaks the shortcut tests that run after this one.
    click_at(10, 20, 1);
    let second = next_click(&pointer, Duration::from_secs(2)).expect("no second click arrived");
    assert_eq!((second.x, second.y, second.button), (10, 20, 1));
}
