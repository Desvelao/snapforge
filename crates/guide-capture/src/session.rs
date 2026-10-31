use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::{
    backend::apply_mode, topmost_at, visible_rect, CaptureBackend, CaptureError, CaptureMode,
    CaptureRequest, Screenshot,
};

/// A click relative to the screenshot it produced; `None` when outside it.
type ClickInShot = Option<(i32, i32)>;

/// Clicks closer together than this count as one (double-click, bounce).
const CLICK_DEBOUNCE: Duration = Duration::from_millis(250);

/// A capture can fail for a moment (a window closing under the click); the session goes on until
/// this many in a row fail, which means something is really wrong.
const MAX_FAILED_IN_A_ROW: usize = 3;

/// Cloneable flag used to cancel a pending capture from another thread.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Waits `delay` (reporting the seconds remaining through `on_tick`), then
/// captures. Cancelling during the countdown returns [`CaptureError::Cancelled`].
pub fn run_capture(
    backend: &dyn CaptureBackend,
    request: &CaptureRequest,
    delay: Duration,
    cancel: &CancelToken,
    mut on_tick: impl FnMut(u64),
) -> Result<Screenshot, CaptureError> {
    let deadline = Instant::now() + delay;
    let mut last_reported = u64::MAX;
    loop {
        if cancel.is_cancelled() {
            return Err(CaptureError::Cancelled);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        if secs != last_reported {
            last_reported = secs;
            on_tick(secs);
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
    if cancel.is_cancelled() {
        return Err(CaptureError::Cancelled);
    }
    backend.capture(request)
}

/// Takes a screenshot for every click received on `clicks` (desktop pixels)
/// until `cancel` is set, `max` shots were taken (`None` = unlimited) or the
/// sender is dropped. Returns the number of shots delivered to `on_shot`.
/// A click whose capture fails is skipped; the error ends the session only after
/// [`MAX_FAILED_IN_A_ROW`] failures in a row.
///
/// `request.mode` decides what each click captures; `CaptureMode::Window(_)`
/// means "the window under the click" (its id is ignored). `on_shot` also gets
/// the click position relative to the screenshot, `None` when the click fell
/// outside of it. Each capture waits `settle` first so the clicked
/// application can react.
pub fn run_click_session(
    backend: &dyn CaptureBackend,
    request: &CaptureRequest,
    clicks: &Receiver<(i32, i32)>,
    cancel: &CancelToken,
    max: Option<usize>,
    settle: Duration,
    mut on_shot: impl FnMut(Screenshot, ClickInShot),
) -> Result<usize, CaptureError> {
    let mut taken = 0;
    let mut failed_in_a_row = 0;
    let mut last_click: Option<Instant> = None;
    while max.is_none_or(|m| taken < m) {
        if cancel.is_cancelled() {
            break;
        }
        let click = match clicks.recv_timeout(Duration::from_millis(50)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let now = Instant::now();
        if last_click.is_some_and(|t| now.duration_since(t) < CLICK_DEBOUNCE) {
            continue;
        }
        last_click = Some(now);
        std::thread::sleep(settle);
        if cancel.is_cancelled() {
            break;
        }
        match capture_for_click(backend, request, click) {
            Ok(Some((shot, at))) => {
                failed_in_a_row = 0;
                on_shot(shot, at);
                taken += 1;
            }
            Ok(None) => failed_in_a_row = 0,
            Err(e) => {
                failed_in_a_row += 1;
                if failed_in_a_row >= MAX_FAILED_IN_A_ROW {
                    return Err(e);
                }
            }
        }
    }
    Ok(taken)
}

/// One screenshot for a click, plus the click relative to it. `None` when
/// there is nothing to capture (window mode with no window under the click).
fn capture_for_click(
    backend: &dyn CaptureBackend,
    request: &CaptureRequest,
    click: (i32, i32),
) -> Result<Option<(Screenshot, ClickInShot)>, CaptureError> {
    let (shot, origin) = match &request.mode {
        CaptureMode::Window(_) => {
            let full = backend.capture(&CaptureRequest {
                mode: CaptureMode::FullScreen,
                include_cursor: request.include_cursor,
            })?;
            let windows = backend.windows()?;
            let Some(w) = topmost_at(&windows, click, (full.width, full.height)) else {
                return Ok(None);
            };
            let Some(v) = visible_rect(w.rect, (full.width, full.height)) else {
                return Ok(None);
            };
            let shot = apply_mode(full, &[], &windows, &CaptureMode::Window(w.id))?;
            (shot, (v.x as i32, v.y as i32))
        }
        mode => {
            let origin = match mode {
                CaptureMode::Region(r) => (r.x as i32, r.y as i32),
                CaptureMode::Monitor(id) => backend
                    .monitors()?
                    .iter()
                    .find(|m| m.id == *id)
                    .map_or((0, 0), |m| (m.x, m.y)),
                _ => (0, 0),
            };
            (backend.capture(request)?, origin)
        }
    };
    let (x, y) = (click.0 - origin.0, click.1 - origin.1);
    let inside = x >= 0 && y >= 0 && x < shot.width as i32 && y < shot.height as i32;
    Ok(Some((shot, inside.then_some((x, y)))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capabilities, IRect, Monitor, Rect, Support, UnavailableBackend, WindowInfo};
    use std::sync::mpsc;

    fn req() -> CaptureRequest {
        CaptureRequest {
            include_cursor: false,
            mode: CaptureMode::FullScreen,
        }
    }

    #[test]
    fn cancel_before_start_returns_cancelled() {
        let b = UnavailableBackend::new("t", "nope");
        let c = CancelToken::default();
        c.cancel();
        let r = run_capture(&b, &req(), Duration::ZERO, &c, |_| {});
        assert_eq!(r, Err(CaptureError::Cancelled));
    }

    #[test]
    fn cancel_during_countdown_stops_early() {
        let b = UnavailableBackend::new("t", "nope");
        let c = CancelToken::default();
        let c2 = c.clone();
        let start = Instant::now();
        let r = run_capture(&b, &req(), Duration::from_secs(5), &c, |_| c2.cancel());
        assert_eq!(r, Err(CaptureError::Cancelled));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn countdown_reports_each_second_then_surfaces_backend_error() {
        let b = UnavailableBackend::new("t", "nope");
        let mut ticks = vec![];
        let r = run_capture(
            &b,
            &req(),
            Duration::from_millis(1500),
            &CancelToken::default(),
            |s| ticks.push(s),
        );
        assert_eq!(ticks, vec![2, 1]);
        assert_eq!(r, Err(CaptureError::Unsupported("nope".into())));
    }

    /// 100x50 desktop with one window and one monitor.
    struct Desktop;

    impl CaptureBackend for Desktop {
        fn capabilities(&self) -> Capabilities {
            let y = Support::Yes;
            Capabilities {
                backend: "fake".into(),
                full_screen: y.clone(),
                monitor_select: y.clone(),
                region: y.clone(),
                window: y.clone(),
                cursor: y,
            }
        }
        fn monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
            Ok(vec![Monitor {
                id: 7,
                name: "m".into(),
                x: 50,
                y: 0,
                width: 50,
                height: 50,
                primary: true,
            }])
        }
        fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
            Ok(vec![WindowInfo {
                id: 3,
                title: "w".into(),
                rect: IRect {
                    x: 10,
                    y: 10,
                    w: 20,
                    h: 20,
                },
            }])
        }
        fn capture(&self, request: &CaptureRequest) -> Result<Screenshot, CaptureError> {
            let full = Screenshot {
                width: 100,
                height: 50,
                rgba: vec![0; 100 * 50 * 4],
            };
            apply_mode(full, &self.monitors()?, &self.windows()?, &request.mode)
        }
    }

    fn session(
        mode: CaptureMode,
        clicks: &[(i32, i32)],
        max: Option<usize>,
    ) -> Vec<(u32, u32, ClickInShot)> {
        let (tx, rx) = mpsc::channel();
        for c in clicks {
            tx.send(*c).unwrap();
        }
        drop(tx);
        let mut out = vec![];
        run_click_session(
            &Desktop,
            &CaptureRequest {
                mode,
                include_cursor: false,
            },
            &rx,
            &CancelToken::default(),
            max,
            Duration::ZERO,
            |s, at| out.push((s.width, s.height, at)),
        )
        .unwrap();
        out
    }

    #[test]
    fn full_screen_clicks_are_reported_in_screenshot_coordinates() {
        let r = session(CaptureMode::FullScreen, &[(5, 6)], None);
        assert_eq!(r, vec![(100, 50, Some((5, 6)))]);
    }

    #[test]
    fn a_monitor_shot_reports_the_click_relative_to_the_monitor() {
        let r = session(CaptureMode::Monitor(7), &[(60, 5)], None);
        assert_eq!(r, vec![(50, 50, Some((10, 5)))]);
    }

    #[test]
    fn a_click_outside_the_region_still_captures_but_has_no_position() {
        let region = CaptureMode::Region(Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        });
        let r = session(region, &[(90, 40)], None);
        assert_eq!(r, vec![(10, 10, None)]);
    }

    #[test]
    fn window_mode_captures_the_window_under_the_click_and_skips_empty_desktop() {
        let r = session(CaptureMode::Window(0), &[(15, 12)], None);
        assert_eq!(r, vec![(20, 20, Some((5, 2)))]);
        assert!(session(CaptureMode::Window(0), &[(90, 40)], None).is_empty());
    }

    #[test]
    fn rapid_clicks_count_once_and_the_limit_ends_the_session() {
        let r = session(CaptureMode::FullScreen, &[(1, 1), (2, 2), (3, 3)], None);
        assert_eq!(r.len(), 1);
        let (tx, rx) = mpsc::channel();
        tx.send((1, 1)).unwrap();
        let taken = run_click_session(
            &Desktop,
            &CaptureRequest {
                mode: CaptureMode::FullScreen,
                include_cursor: false,
            },
            &rx,
            &CancelToken::default(),
            Some(1),
            Duration::ZERO,
            |_, _| {},
        );
        assert_eq!(taken, Ok(1));
        drop(tx);
    }

    #[test]
    fn cancelling_stops_a_session_that_is_waiting_for_clicks() {
        let (_tx, rx) = mpsc::channel();
        let cancel = CancelToken::default();
        cancel.cancel();
        let taken = run_click_session(
            &Desktop,
            &CaptureRequest {
                mode: CaptureMode::FullScreen,
                include_cursor: false,
            },
            &rx,
            &cancel,
            None,
            Duration::ZERO,
            |_, _| {},
        );
        assert_eq!(taken, Ok(0));
    }

    /// Fails the first `failures` captures, then behaves like [`Desktop`].
    struct Flaky {
        failures: std::sync::atomic::AtomicUsize,
    }

    impl CaptureBackend for Flaky {
        fn capabilities(&self) -> Capabilities {
            Desktop.capabilities()
        }
        fn monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
            Desktop.monitors()
        }
        fn capture(&self, request: &CaptureRequest) -> Result<Screenshot, CaptureError> {
            let left = self.failures.load(Ordering::SeqCst);
            if left > 0 {
                self.failures.store(left - 1, Ordering::SeqCst);
                return Err(CaptureError::Backend("window is gone".into()));
            }
            Desktop.capture(request)
        }
    }

    /// Runs a session over `clicks`, sent one debounce period apart so none is merged with another.
    fn flaky_session(failures: usize, clicks: usize) -> (Result<usize, CaptureError>, usize) {
        let backend = Flaky {
            failures: failures.into(),
        };
        let (tx, rx) = mpsc::channel();
        let sender = std::thread::spawn(move || {
            for i in 0..clicks {
                tx.send((i as i32, 0)).unwrap();
                std::thread::sleep(CLICK_DEBOUNCE + Duration::from_millis(50));
            }
        });
        let mut shots = 0;
        let result = run_click_session(
            &backend,
            &req(),
            &rx,
            &CancelToken::default(),
            None,
            Duration::from_millis(10),
            |_, _| shots += 1,
        );
        sender.join().unwrap();
        (result, shots)
    }

    #[test]
    fn one_failed_capture_skips_that_click_and_the_session_goes_on() {
        assert_eq!(flaky_session(1, 2), (Ok(1), 1));
    }

    #[test]
    fn repeated_failures_end_the_session_with_the_error() {
        let (result, shots) = flaky_session(MAX_FAILED_IN_A_ROW, MAX_FAILED_IN_A_ROW + 1);
        assert_eq!(result, Err(CaptureError::Backend("window is gone".into())));
        assert_eq!(shots, 0);
    }
}
