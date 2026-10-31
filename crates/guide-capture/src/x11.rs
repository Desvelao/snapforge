use std::{collections::HashMap, sync::Mutex};

use x11rb::{
    connection::Connection,
    cookie::Cookie,
    protocol::{
        randr::ConnectionExt as _,
        xfixes::ConnectionExt as _,
        xproto::{
            Atom, AtomEnum, ConnectionExt as _, GetGeometryReply, GetPropertyReply, ImageFormat,
            ImageOrder, TranslateCoordinatesReply, Window,
        },
    },
    rust_connection::RustConnection,
};

use crate::{
    backend::apply_mode, composite_cursor, frame_rect, Capabilities, CaptureBackend, CaptureError,
    CaptureMode, CaptureRequest, CursorImage, Monitor, Screenshot, Support, WindowInfo,
};

/// `_NET_WM_DESKTOP` value meaning "visible on every desktop".
const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;

type PropertyCookie<'c> = Cookie<'c, RustConnection, GetPropertyReply>;
/// Value bytes, format (8/16/32) and type atom of a property.
type Property = (Vec<u8>, u8, Atom);

/// The requests sent for one window while listing windows.
struct Pending<'c> {
    win: Window,
    state: Option<PropertyCookie<'c>>,
    desktop: Option<PropertyCookie<'c>>,
    extents: Option<PropertyCookie<'c>>,
    names: [Option<PropertyCookie<'c>>; 2],
    geometry: Result<Cookie<'c, RustConnection, GetGeometryReply>, CaptureError>,
    position: Result<Cookie<'c, RustConnection, TranslateCoordinatesReply>, CaptureError>,
}

fn read_property(cookie: Option<PropertyCookie<'_>>) -> Option<Property> {
    let r = cookie?.reply().ok()?;
    (r.value_len > 0).then_some((r.value, r.format, r.type_))
}

fn words(property: Option<Property>) -> Option<Vec<u32>> {
    let (bytes, format, _) = property?;
    (format == 32).then(|| {
        bytes
            .chunks_exact(4)
            .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    })
}

/// The monitor to offer when the X server cannot list monitors: the whole screen.
fn whole_screen_monitor(width: u16, height: u16) -> Monitor {
    Monitor {
        id: 0,
        name: "Screen".into(),
        x: 0,
        y: 0,
        width: width.into(),
        height: height.into(),
        primary: true,
    }
}

/// `_NET_WM_NAME` is UTF-8; the old `WM_NAME` is usually type STRING, which is Latin-1.
fn decode_title(bytes: &[u8], latin1: bool) -> String {
    if latin1 {
        bytes.iter().map(|&b| char::from(b)).collect()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

pub struct X11Backend {
    conn: RustConnection,
    screen: usize,
    /// Atoms are fixed for the life of the connection, so each name is asked for once.
    atoms: Mutex<HashMap<String, Atom>>,
}

impl X11Backend {
    pub fn connect() -> Result<Self, CaptureError> {
        let (conn, screen) = x11rb::connect(None)
            .map_err(|e| CaptureError::Backend(format!("cannot connect to the X server: {e}")))?;
        Ok(Self {
            conn,
            screen,
            atoms: Mutex::default(),
        })
    }

    fn root(&self) -> Window {
        self.conn.setup().roots[self.screen].root
    }

    fn atom(&self, name: &str) -> Option<Atom> {
        if let Some(atom) = self.atoms.lock().ok()?.get(name) {
            return Some(*atom);
        }
        let atom = self
            .conn
            .intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom;
        self.atoms.lock().ok()?.insert(name.to_string(), atom);
        Some(atom)
    }

    /// Asks for a property without waiting for the answer, so many can be in flight at once.
    fn request_property(&self, win: Window, name: &str) -> Option<PropertyCookie<'_>> {
        let atom = self.atom(name)?;
        self.conn
            .get_property(false, win, atom, AtomEnum::ANY, 0, 1 << 16)
            .ok()
    }

    /// Raw property bytes, or `None` when absent/unreadable.
    fn property(&self, win: Window, name: &str) -> Option<Property> {
        read_property(self.request_property(win, name))
    }

    fn property32(&self, win: Window, name: &str) -> Option<Vec<u32>> {
        words(self.property(win, name))
    }

    /// The first of `_NET_WM_NAME` and `WM_NAME` that the window has.
    fn title_from(&self, names: [Option<PropertyCookie<'_>>; 2]) -> String {
        let string = self.atom("STRING");
        let found = names.into_iter().find_map(read_property);
        found
            .map(|(bytes, _, type_)| decode_title(&bytes, Some(type_) == string))
            .unwrap_or_default()
    }

    /// Why the pointer cannot be captured, or `None` when XFixes works.
    fn cursor_unavailable(&self) -> Option<String> {
        let ok = self
            .conn
            .xfixes_query_version(4, 0)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some();
        (!ok).then(|| "the X server does not support the XFixes extension".to_string())
    }

    fn cursor_image(&self) -> Result<CursorImage, CaptureError> {
        if let Some(why) = self.cursor_unavailable() {
            return Err(CaptureError::Unsupported(why));
        }
        let r = self
            .conn
            .xfixes_get_cursor_image()
            .map_err(|e| CaptureError::Backend(e.to_string()))?
            .reply()
            .map_err(|e| CaptureError::Backend(format!("cannot read the cursor: {e}")))?;
        Ok(CursorImage {
            x: r.x.into(),
            y: r.y.into(),
            width: r.width.into(),
            height: r.height.into(),
            xhot: r.xhot.into(),
            yhot: r.yhot.into(),
            argb: r.cursor_image,
        })
    }

    fn root_image(&self) -> Result<Screenshot, CaptureError> {
        let err = |e: &dyn std::fmt::Display| CaptureError::Backend(e.to_string());
        let setup = self.conn.setup();
        let screen = &setup.roots[self.screen];
        let fmt = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == screen.root_depth)
            .ok_or_else(|| CaptureError::Backend("no pixmap format for root depth".into()))?;
        if fmt.bits_per_pixel != 32 || setup.image_byte_order != ImageOrder::LSB_FIRST {
            return Err(CaptureError::Backend(format!(
                "unsupported X11 pixel format ({} bpp)",
                fmt.bits_per_pixel
            )));
        }
        // The size in `setup()` is from connection time; RandR can change it while we run.
        let geometry = self
            .conn
            .get_geometry(screen.root)
            .map_err(|e| err(&e))?
            .reply()
            .map_err(|e| err(&e))?;
        let (w, h) = (geometry.width, geometry.height);
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, screen.root, 0, 0, w, h, !0)
            .map_err(|e| err(&e))?
            .reply()
            .map_err(|e| err(&e))?;
        let mut rgba = reply.data;
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2); // BGRX -> RGBX
            px[3] = 255;
        }
        Ok(Screenshot {
            width: w.into(),
            height: h.into(),
            rgba,
        })
    }
}

impl CaptureBackend for X11Backend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            backend: "X11".into(),
            full_screen: Support::Yes,
            monitor_select: Support::Yes,
            region: Support::Yes,
            window: if self
                .property32(self.root(), "_NET_CLIENT_LIST_STACKING")
                .is_some()
            {
                Support::Yes
            } else {
                Support::No(
                    "the window manager does not provide a window list (_NET_CLIENT_LIST_STACKING)"
                        .into(),
                )
            },
            cursor: match self.cursor_unavailable() {
                None => Support::Yes,
                Some(why) => Support::No(why),
            },
        }
    }

    fn monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
        let root = self.conn.setup().roots[self.screen].root;
        let reply = self
            .conn
            .randr_get_monitors(root, true)
            .ok()
            .and_then(|c| c.reply().ok());
        let Some(reply) = reply else {
            // No RandR (or it failed): the whole screen is the one monitor.
            let g = self
                .conn
                .get_geometry(root)
                .map_err(|e| CaptureError::Backend(e.to_string()))?
                .reply()
                .map_err(|e| CaptureError::Backend(e.to_string()))?;
            return Ok(vec![whole_screen_monitor(g.width, g.height)]);
        };
        reply
            .monitors
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let name = self
                    .conn
                    .get_atom_name(m.name)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                    .unwrap_or_else(|| format!("Monitor {}", i + 1));
                Ok(Monitor {
                    // The atom of the monitor's name: it does not change when monitors are plugged
                    // in, out or reordered (the position in the list does).
                    id: m.name,
                    name,
                    x: m.x.into(),
                    y: m.y.into(),
                    width: m.width.into(),
                    height: m.height.into(),
                    primary: m.primary,
                })
            })
            .collect()
    }

    fn pointer_position(&self) -> Option<(i32, i32)> {
        let r = self.conn.query_pointer(self.root()).ok()?.reply().ok()?;
        Some((i32::from(r.root_x), i32::from(r.root_y)))
    }

    fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
        let root = self.root();
        let stacking = self
            .property32(root, "_NET_CLIENT_LIST_STACKING")
            .ok_or_else(|| {
                CaptureError::Unsupported(
                    "the window manager does not provide a window list (_NET_CLIENT_LIST_STACKING)"
                        .into(),
                )
            })?;
        let current_desktop = self
            .property32(root, "_NET_CURRENT_DESKTOP")
            .and_then(|v| v.first().copied());
        let hidden = self.atom("_NET_WM_STATE_HIDDEN");
        let backend_err = |e: &dyn std::fmt::Display| CaptureError::Backend(e.to_string());

        // Send every request for every window first and read the answers afterwards, so the
        // round trips overlap instead of costing a full wait each.
        let pending: Vec<_> = stacking
            .into_iter()
            .map(|win| Pending {
                win,
                state: self.request_property(win, "_NET_WM_STATE"),
                desktop: self.request_property(win, "_NET_WM_DESKTOP"),
                extents: self.request_property(win, "_NET_FRAME_EXTENTS"),
                names: ["_NET_WM_NAME", "WM_NAME"].map(|n| self.request_property(win, n)),
                geometry: self.conn.get_geometry(win).map_err(|e| backend_err(&e)),
                position: self
                    .conn
                    .translate_coordinates(win, root, 0, 0)
                    .map_err(|e| backend_err(&e)),
            })
            .collect();

        let mut out = Vec::new();
        for p in pending {
            let minimized = words(read_property(p.state))
                .zip(hidden)
                .is_some_and(|(state, h)| state.contains(&h));
            let other_desktop = words(read_property(p.desktop))
                .and_then(|v| v.first().copied())
                .is_some_and(|d| d != ALL_DESKTOPS && Some(d) != current_desktop);
            let extents = words(read_property(p.extents))
                .filter(|v| v.len() == 4)
                .map_or([0; 4], |v| [v[0], v[1], v[2], v[3]]);
            let title = self.title_from(p.names);
            // A window can vanish between listing and querying; skip it then.
            let (Ok(geo), Ok(pos)) = (p.geometry?.reply(), p.position?.reply()) else {
                continue;
            };
            if minimized || other_desktop {
                continue;
            }
            out.push(WindowInfo {
                id: p.win,
                title,
                rect: frame_rect(
                    (pos.dst_x.into(), pos.dst_y.into()),
                    (geo.width.into(), geo.height.into()),
                    extents,
                ),
            });
        }
        Ok(out)
    }

    fn capture(&self, request: &CaptureRequest) -> Result<Screenshot, CaptureError> {
        let mut full = self.root_image()?;
        if request.include_cursor {
            // Blend before cropping so region/window/monitor crops keep the pointer.
            composite_cursor(&mut full, &self.cursor_image()?);
        }
        let monitors = match request.mode {
            CaptureMode::Monitor(_) => self.monitors()?,
            _ => Vec::new(),
        };
        let windows = match request.mode {
            CaptureMode::Window(_) => self.windows()?,
            _ => Vec::new(),
        };
        apply_mode(full, &monitors, &windows, &request.mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_randr_the_whole_screen_is_the_one_monitor() {
        let m = whole_screen_monitor(1920, 1080);
        assert_eq!((m.x, m.y, m.width, m.height), (0, 0, 1920, 1080));
        assert!(m.primary);
    }

    #[test]
    fn a_latin1_title_keeps_its_accented_letters() {
        assert_eq!(
            decode_title(&[0x4d, 0xfc, 0x6e, 0x63, 0x68, 0x65, 0x6e], true),
            "München"
        );
    }

    #[test]
    fn a_utf8_title_is_decoded_as_utf8() {
        assert_eq!(decode_title("Münchén ✓".as_bytes(), false), "Münchén ✓");
    }
}
