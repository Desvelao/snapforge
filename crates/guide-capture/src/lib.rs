//! Screenshot capture behind a UI-independent interface.
//!
//! The UI only sees [`CaptureBackend`], [`CaptureRequest`] and [`Screenshot`];
//! X11/Wayland details stay inside the backend modules.

mod backend;
mod cursor;
mod region;
mod repeat;
mod session;
mod windows;
mod x11;

pub use backend::{detect_backend, CaptureBackend, UnavailableBackend};
pub use cursor::{composite_cursor, CursorImage};
pub use region::region_from_drag;
pub use repeat::{resolve_repeat, LastCapture};
pub use session::{run_capture, run_click_session, CancelToken};
pub use windows::{frame_rect, topmost_at, visible_rect, IRect, WindowInfo};

/// Raw RGBA8 pixels, row-major, no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Screenshot {
    /// A copy shrunk (box filter) so neither side exceeds `max_side`, for
    /// showing on screen when the full image is larger than the GPU's texture
    /// limit. `None` when the image already fits. Never use it for saving.
    pub fn scaled_to_fit(&self, max_side: u32) -> Option<Screenshot> {
        let longest = self.width.max(self.height);
        if max_side == 0 || longest <= max_side || self.width == 0 || self.height == 0 {
            return None;
        }
        let factor = longest.div_ceil(max_side);
        let (w, h) = (self.width.div_ceil(factor), self.height.div_ceil(factor));
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for dy in 0..h {
            for dx in 0..w {
                let (x0, y0) = (dx * factor, dy * factor);
                let (x1, y1) = (
                    (x0 + factor).min(self.width),
                    (y0 + factor).min(self.height),
                );
                let mut sum = [0u32; 4];
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = (y as usize * self.width as usize + x as usize) * 4;
                        for (c, s) in sum.iter_mut().enumerate() {
                            *s += u32::from(self.rgba[i + c]);
                        }
                    }
                }
                let n = (x1 - x0) * (y1 - y0);
                rgba.extend(sum.iter().map(|s| ((s + n / 2) / n) as u8));
            }
        }
        Some(Screenshot {
            width: w,
            height: h,
            rgba,
        })
    }

    /// Returns the sub-image at `rect`, or an error if it is empty or out of bounds.
    pub fn crop(&self, rect: Rect) -> Result<Screenshot, CaptureError> {
        let in_bounds = rect.w > 0
            && rect.h > 0
            && rect.x.checked_add(rect.w).is_some_and(|r| r <= self.width)
            && rect.y.checked_add(rect.h).is_some_and(|b| b <= self.height);
        if !in_bounds {
            return Err(CaptureError::InvalidRegion(rect));
        }
        let mut rgba = Vec::with_capacity(rect.w as usize * rect.h as usize * 4);
        for row in rect.y..rect.y + rect.h {
            let start = (row as usize * self.width as usize + rect.x as usize) * 4;
            rgba.extend_from_slice(&self.rgba[start..start + rect.w as usize * 4]);
        }
        Ok(Screenshot {
            width: rect.w,
            height: rect.h,
            rgba,
        })
    }
}

/// Pixel rectangle in screenshot coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub id: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    FullScreen,
    Monitor(u32),
    Region(Rect),
    /// A top-level window, identified by [`WindowInfo::id`]; includes its frame.
    Window(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRequest {
    pub mode: CaptureMode,
    /// Draw the mouse pointer into the screenshot. Backends that cannot do
    /// this fail with [`CaptureError::Unsupported`] instead of omitting it.
    pub include_cursor: bool,
}

/// A feature is either available or unavailable with a user-facing reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    Yes,
    No(String),
}

impl Support {
    pub fn is_yes(&self) -> bool {
        matches!(self, Support::Yes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub backend: String,
    pub full_screen: Support,
    pub monitor_select: Support,
    pub region: Support,
    pub window: Support,
    pub cursor: Support,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CaptureError {
    #[error("capture cancelled")]
    Cancelled,
    #[error("{0}")]
    Unsupported(String),
    #[error("invalid region {0:?}")]
    InvalidRegion(Rect),
    #[error("no visible window with id {0}")]
    NoSuchWindow(u32),
    #[error("no monitor with id {0}")]
    NoSuchMonitor(u32),
    #[error("capture failed: {0}")]
    Backend(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img() -> Screenshot {
        // 3x2, pixel value = index so crops are checkable
        Screenshot {
            width: 3,
            height: 2,
            rgba: (0..6u8).flat_map(|i| [i, i, i, 255]).collect(),
        }
    }

    #[test]
    fn scaled_to_fit_leaves_images_that_fit_alone() {
        assert_eq!(img().scaled_to_fit(3), None);
        assert_eq!(img().scaled_to_fit(100), None);
        assert_eq!(img().scaled_to_fit(0), None);
    }

    #[test]
    fn scaled_to_fit_averages_blocks_and_keeps_aspect() {
        // 4x2: left half black, right half white, shrunk to at most 2 wide.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 255 };
                rgba.extend([v, v, v, 255]);
            }
        }
        let s = Screenshot {
            width: 4,
            height: 2,
            rgba,
        }
        .scaled_to_fit(2)
        .unwrap();
        assert_eq!((s.width, s.height), (2, 1));
        assert_eq!(s.rgba, vec![0, 0, 0, 255, 255, 255, 255, 255]);
    }

    #[test]
    fn scaled_to_fit_handles_odd_sizes_and_averages() {
        // 3x1 -> factor 2 -> 2x1; last block covers a single pixel.
        let s = Screenshot {
            width: 3,
            height: 1,
            rgba: vec![0, 0, 0, 255, 100, 100, 100, 255, 40, 40, 40, 255],
        }
        .scaled_to_fit(2)
        .unwrap();
        assert_eq!((s.width, s.height), (2, 1));
        assert_eq!(s.rgba, vec![50, 50, 50, 255, 40, 40, 40, 255]);
    }

    #[test]
    fn scaled_to_fit_result_fits_for_huge_desktops() {
        // Wider than a common 8192 GPU texture limit (a dual-5K desktop is 10240 wide).
        let big = Screenshot {
            width: 11520,
            height: 2160,
            rgba: vec![7; 11520 * 2160 * 4],
        };
        let s = big.scaled_to_fit(8192).unwrap();
        assert!(s.width <= 8192 && s.height <= 8192);
        assert_eq!(s.rgba.len(), s.width as usize * s.height as usize * 4);
        assert!(s.rgba.iter().all(|b| *b == 7));
    }

    #[test]
    fn crop_extracts_expected_pixels() {
        let c = img()
            .crop(Rect {
                x: 1,
                y: 1,
                w: 2,
                h: 1,
            })
            .unwrap();
        assert_eq!((c.width, c.height), (2, 1));
        assert_eq!(c.rgba, vec![4, 4, 4, 255, 5, 5, 5, 255]);
    }

    #[test]
    fn crop_rejects_out_of_bounds_and_empty() {
        for r in [
            Rect {
                x: 2,
                y: 0,
                w: 2,
                h: 1,
            },
            Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 1,
            },
            Rect {
                x: u32::MAX,
                y: 0,
                w: 2,
                h: 1,
            },
        ] {
            assert_eq!(img().crop(r), Err(CaptureError::InvalidRegion(r)));
        }
    }
}
