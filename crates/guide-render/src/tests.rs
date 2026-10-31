use guide_core::{Document, Editor, Kind, ObscureMode, PixelRect, Point, RectF, Rgba, Style};

use super::*;

fn solid(w: u32, h: u32, v: u8) -> Image {
    Image {
        width: w,
        height: h,
        rgba: [v, v, v, 255].repeat(w as usize * h as usize),
    }
}

fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = (y as usize * img.width as usize + x as usize) * 4;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

fn from_fn(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> Image {
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let v = f(x, y);
            rgba.extend([v, v, v, 255]);
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

fn r(x: f32, y: f32, w: f32, h: f32) -> RectF {
    RectF { x, y, w, h }
}

fn style(color: [u8; 4], width: f32, fill: Option<[u8; 4]>) -> Style {
    Style {
        color: Rgba(color),
        width,
        fill: fill.map(Rgba),
    }
}

fn doc_with(items: Vec<(Kind, Style)>) -> Document {
    let mut e = Editor::new(Document::default());
    for (k, s) in items {
        e.add(k, s);
    }
    e.document().clone()
}

const BLUE: [u8; 4] = [0, 0, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GRAY: [u8; 4] = [100, 100, 100, 255];

#[test]
fn rectangle_fill_covers_inside_only_and_the_original_is_untouched() {
    let original = solid(40, 40, 100);
    let backup = original.clone();
    let doc = doc_with(vec![(
        Kind::Rectangle {
            rect: r(10.0, 10.0, 20.0, 20.0),
        },
        style(BLUE, 0.0, Some(BLUE)),
    )]);
    let out = render(&original, &doc).unwrap();
    assert_eq!(px(&out, 20, 20), BLUE);
    assert_eq!(px(&out, 5, 5), GRAY);
    assert_eq!(px(&out, 35, 20), GRAY);
    assert_eq!(
        original, backup,
        "the original image must never be modified"
    );
}

#[test]
fn rectangle_outline_leaves_the_inside_alone() {
    let doc = doc_with(vec![(
        Kind::Rectangle {
            rect: r(10.0, 10.0, 20.0, 20.0),
        },
        style(RED, 4.0, None),
    )]);
    let out = render(&solid(40, 40, 100), &doc).unwrap();
    assert_eq!(px(&out, 20, 20), GRAY, "interior stays untouched");
    assert_eq!(px(&out, 10, 20), RED, "the outline is drawn on the edge");
}

#[test]
fn ellipse_fills_the_centre_but_not_the_corners() {
    let doc = doc_with(vec![(
        Kind::Ellipse {
            rect: r(0.0, 0.0, 40.0, 40.0),
        },
        style(BLUE, 0.0, Some(BLUE)),
    )]);
    let out = render(&solid(40, 40, 100), &doc).unwrap();
    assert_eq!(px(&out, 20, 20), BLUE);
    assert_eq!(px(&out, 1, 1), GRAY);
}

#[test]
fn line_arrow_and_pen_draw_where_expected() {
    let line = (
        Kind::Line {
            from: Point::new(5.0, 20.0),
            to: Point::new(35.0, 20.0),
        },
        style(RED, 4.0, None),
    );
    let out = render(&solid(60, 60, 100), &doc_with(vec![line])).unwrap();
    assert_eq!(px(&out, 20, 20), RED);
    assert_eq!(px(&out, 20, 40), GRAY);

    let arrow = (
        Kind::Arrow {
            from: Point::new(5.0, 30.0),
            to: Point::new(55.0, 30.0),
        },
        style(RED, 4.0, None),
    );
    let out = render(&solid(60, 60, 100), &doc_with(vec![arrow])).unwrap();
    assert_eq!(px(&out, 30, 30), RED, "shaft");
    assert_eq!(px(&out, 52, 30), RED, "head");
    assert_eq!(px(&out, 52, 20), GRAY, "head is narrow, not a block");

    let pen = (
        Kind::Pen {
            points: vec![
                Point::new(10.0, 10.0),
                Point::new(30.0, 40.0),
                Point::new(50.0, 10.0),
            ],
        },
        style(RED, 4.0, None),
    );
    let out = render(&solid(60, 60, 100), &doc_with(vec![pen])).unwrap();
    assert_eq!(px(&out, 30, 40), RED);
}

#[test]
fn degenerate_shapes_do_not_panic_or_draw_garbage() {
    let s = style(RED, 4.0, None);
    let doc = doc_with(vec![
        (Kind::Pen { points: vec![] }, s.clone()),
        (
            Kind::Pen {
                points: vec![Point::new(20.0, 20.0)],
            },
            s.clone(),
        ),
        (
            Kind::Arrow {
                from: Point::new(5.0, 5.0),
                to: Point::new(5.0, 5.0),
            },
            s.clone(),
        ),
        (
            Kind::Line {
                from: Point::new(9.0, 9.0),
                to: Point::new(9.0, 9.0),
            },
            s.clone(),
        ),
        (
            Kind::Rectangle {
                rect: r(5.0, 5.0, 0.0, 0.0),
            },
            s.clone(),
        ),
        (
            Kind::Ellipse {
                rect: r(5.0, 5.0, 0.0, 10.0),
            },
            s.clone(),
        ),
        (
            Kind::Obscure {
                rect: r(100.0, 100.0, 10.0, 10.0),
                mode: ObscureMode::Blur,
                strength: 5,
            },
            s,
        ),
    ]);
    let out = render(&solid(40, 40, 100), &doc).unwrap();
    assert_eq!(px(&out, 20, 20), RED, "single-point pen stroke is a dot");
    assert_eq!(px(&out, 35, 35), GRAY);
}

#[test]
fn highlighter_tints_light_pixels_but_keeps_dark_text_readable() {
    let mut original = solid(60, 20, 255);
    // A black "text" pixel block in the middle of the highlighted band.
    for y in 8..12 {
        for x in 28..32 {
            let i = (y * 60 + x) * 4;
            original.rgba[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
        }
    }
    let hl = (
        Kind::Highlighter {
            points: vec![Point::new(5.0, 10.0), Point::new(55.0, 10.0)],
        },
        style([255, 235, 0, 255], 12.0, None),
    );
    let out = render(&original, &doc_with(vec![hl])).unwrap();
    assert_eq!(
        px(&out, 10, 10),
        [255, 235, 0, 255],
        "background becomes yellow"
    );
    assert_eq!(px(&out, 30, 10), [0, 0, 0, 255], "black text stays black");
    assert_eq!(
        px(&out, 10, 2),
        [255, 255, 255, 255],
        "outside the band is untouched"
    );
}

#[test]
fn text_renders_ink_inside_its_measured_box_and_bold_adds_ink() {
    let make = |bold| {
        doc_with(vec![(
            Kind::Text {
                at: Point::new(10.0, 10.0),
                text: "Hello".into(),
                size: 24.0,
                bold,
            },
            style([0, 0, 0, 255], 0.0, None),
        )])
    };
    let white = solid(160, 60, 255);
    let ink = |img: &Image| img.rgba.chunks_exact(4).filter(|p| p[0] < 128).count();
    let normal = render(&white, &make(false)).unwrap();
    let bold = render(&white, &make(true)).unwrap();
    assert!(ink(&normal) > 40, "text produced no ink");
    assert!(ink(&bold) > ink(&normal), "bold should add ink");

    let (w, h) = measure_text("Hello", 24.0);
    assert!(w > 30.0 && h > 20.0, "{w}x{h}");
    assert!(measure_text("Hello", 48.0).0 > w * 1.8);
    // No ink well outside the measured box (allow a few pixels for overhang).
    for y in 0..60u32 {
        for x in 0..160u32 {
            let outside =
                (x as f32) > 10.0 + w + 6.0 || (y as f32) > 10.0 + h + 6.0 || x < 6 || y < 6;
            if outside {
                assert_eq!(
                    px(&normal, x, y),
                    [255, 255, 255, 255],
                    "stray ink at ({x},{y})"
                );
            }
        }
    }
}

#[test]
fn multiline_text_is_taller_and_text_background_fills() {
    let (_, h1) = measure_text("a", 20.0);
    let (_, h2) = measure_text("a\nb", 20.0);
    assert!(h2 > h1 * 1.8);
    let doc = doc_with(vec![(
        Kind::Text {
            at: Point::new(20.0, 20.0),
            text: "x".into(),
            size: 20.0,
            bold: false,
        },
        style([0, 0, 0, 255], 0.0, Some([255, 255, 0, 255])),
    )]);
    let out = render(&solid(80, 60, 255), &doc).unwrap();
    assert_eq!(
        px(&out, 17, 17),
        [255, 255, 0, 255],
        "padding area is filled"
    );
}

#[test]
fn marker_draws_a_coloured_disc_with_a_white_number() {
    let doc = doc_with(vec![(
        Kind::Marker {
            center: Point::new(30.0, 30.0),
            number: 7,
            radius: 16.0,
        },
        style(RED, 0.0, None),
    )]);
    let out = render(&solid(60, 60, 100), &doc).unwrap();
    assert_eq!(
        px(&out, 30 + 12, 30),
        RED,
        "disc colour away from the digit"
    );
    assert_eq!(px(&out, 30 + 14, 30 - 14), GRAY, "outside the circle");
    let white_inside = (22..38u32)
        .flat_map(|y| (22..38u32).map(move |x| (x, y)))
        .filter(|&(x, y)| px(&out, x, y)[..3].iter().all(|c| *c > 200))
        .count();
    assert!(
        white_inside > 8,
        "the number should be drawn in white, found {white_inside} white pixels"
    );
}

#[test]
fn pixelate_makes_every_block_uniform_and_destroys_the_pattern() {
    // Alternating 1px black/white columns: stands in for small text.
    let original = from_fn(64, 64, |x, _| if x % 2 == 0 { 0 } else { 255 });
    let doc = doc_with(vec![(
        Kind::Obscure {
            rect: r(8.0, 8.0, 32.0, 32.0),
            mode: ObscureMode::Pixelate,
            strength: 8,
        },
        Style::default(),
    )]);
    let out = render(&original, &doc).unwrap();
    for by in 0..4u32 {
        for bx in 0..4u32 {
            let first = px(&out, 8 + bx * 8, 8 + by * 8);
            for y in 0..8 {
                for x in 0..8 {
                    assert_eq!(
                        px(&out, 8 + bx * 8 + x, 8 + by * 8 + y),
                        first,
                        "block ({bx},{by}) is not uniform"
                    );
                }
            }
            assert!(
                (i32::from(first[0]) - 127).abs() <= 1,
                "block should be the average, got {}",
                first[0]
            );
        }
    }
    assert_eq!(
        px(&out, 7, 20),
        px(&original, 7, 20),
        "outside the region is untouched"
    );
    assert_eq!(px(&out, 40, 20), px(&original, 40, 20));
    assert_eq!(px(&out, 41, 20), px(&original, 41, 20));
}

#[test]
fn blur_flattens_a_checkerboard_so_it_cannot_be_read_back() {
    let original = from_fn(80, 80, |x, y| if (x + y) % 2 == 0 { 0 } else { 255 });
    let doc = doc_with(vec![(
        Kind::Obscure {
            rect: r(10.0, 10.0, 50.0, 50.0),
            mode: ObscureMode::Blur,
            strength: 6,
        },
        Style::default(),
    )]);
    let out = render(&original, &doc).unwrap();
    // Away from the region edges the 1px pattern must be gone: neighbours nearly equal, and not 0/255.
    let mut max_step = 0i32;
    for y in 20..50u32 {
        for x in 20..49u32 {
            max_step = max_step
                .max((i32::from(px(&out, x, y)[0]) - i32::from(px(&out, x + 1, y)[0])).abs());
            let v = px(&out, x, y)[0];
            assert!(
                (100..=155).contains(&v),
                "pixel ({x},{y}) = {v} still looks like the original pattern"
            );
        }
    }
    assert!(max_step <= 12, "blur left a step of {max_step}");
    assert_eq!(px(&out, 5, 5), px(&original, 5, 5));
    assert_eq!(
        px(&out, 60, 30),
        px(&original, 60, 30),
        "pixel just outside the region is untouched"
    );
}

#[test]
fn obscure_hides_what_is_below_it_but_not_what_is_drawn_after() {
    let blue_box = (
        Kind::Rectangle {
            rect: r(0.0, 0.0, 64.0, 64.0),
        },
        style(BLUE, 0.0, Some(BLUE)),
    );
    let obscure = (
        Kind::Obscure {
            rect: r(16.0, 16.0, 32.0, 32.0),
            mode: ObscureMode::Pixelate,
            strength: 32,
        },
        Style::default(),
    );
    let white_box = (
        Kind::Rectangle {
            rect: r(20.0, 20.0, 6.0, 6.0),
        },
        style([255, 255, 255, 255], 0.0, Some([255, 255, 255, 255])),
    );
    let original = from_fn(64, 64, |x, _| if x % 2 == 0 { 0 } else { 255 });
    let out = render(&original, &doc_with(vec![blue_box, obscure, white_box])).unwrap();
    assert_eq!(
        px(&out, 40, 40),
        BLUE,
        "obscuring a uniform blue area keeps it blue"
    );
    assert_eq!(
        px(&out, 22, 22),
        [255, 255, 255, 255],
        "annotations added after the blur stay sharp"
    );
}

#[test]
fn crop_cuts_the_output_and_keeps_annotations_aligned() {
    let original = from_fn(60, 40, |x, y| (x * 3 + y) as u8);
    let mut doc = doc_with(vec![(
        Kind::Rectangle {
            rect: r(10.0, 5.0, 4.0, 4.0),
        },
        style(BLUE, 0.0, Some(BLUE)),
    )]);
    doc.crop = Some(PixelRect {
        x: 10,
        y: 5,
        w: 20,
        h: 10,
    });
    let out = render(&original, &doc).unwrap();
    assert_eq!((out.width, out.height), (20, 10));
    assert_eq!(
        px(&out, 0, 0),
        BLUE,
        "annotation at the crop origin lands at (0,0)"
    );
    assert_eq!(
        px(&out, 10, 5),
        px(&original, 20, 10),
        "pixels keep their content"
    );

    let full = render_with(
        &original,
        &doc,
        &RenderOptions {
            apply_crop: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!((full.width, full.height), (60, 40));
}

#[test]
fn excluded_annotation_is_skipped() {
    let doc = doc_with(vec![(
        Kind::Rectangle {
            rect: r(0.0, 0.0, 10.0, 10.0),
        },
        style(BLUE, 0.0, Some(BLUE)),
    )]);
    let id = doc.annotations[0].id;
    let out = render_with(
        &solid(20, 20, 100),
        &doc,
        &RenderOptions {
            exclude: Some(id),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(px(&out, 5, 5), GRAY);
}

#[test]
fn bad_inputs_are_rejected_without_allocating() {
    let huge = Image {
        width: 20_000,
        height: 20_000,
        rgba: Vec::new(),
    };
    assert_eq!(
        render(&huge, &Document::default()),
        Err(RenderError::TooLarge(20_000, 20_000))
    );
    let short = Image {
        width: 4,
        height: 4,
        rgba: vec![0; 10],
    };
    assert_eq!(
        render(&short, &Document::default()),
        Err(RenderError::InvalidImage)
    );
    let empty = Image {
        width: 0,
        height: 4,
        rgba: vec![],
    };
    assert_eq!(
        render(&empty, &Document::default()),
        Err(RenderError::InvalidImage)
    );

    let nan = doc_with(vec![(
        Kind::Rectangle {
            rect: r(f32::NAN, 0.0, 5.0, 5.0),
        },
        Style::default(),
    )]);
    assert!(matches!(
        render(&solid(10, 10, 0), &nan),
        Err(RenderError::InvalidDocument(_))
    ));
    let mut bad_crop = Document::default();
    bad_crop.crop = Some(PixelRect {
        x: 5,
        y: 5,
        w: 20,
        h: 20,
    });
    assert!(matches!(
        render(&solid(10, 10, 0), &bad_crop),
        Err(RenderError::InvalidDocument(_))
    ));
}

#[test]
fn transparent_pixels_survive_the_premultiply_round_trip() {
    let mut img = solid(4, 1, 0);
    img.rgba[..4].copy_from_slice(&[200, 100, 50, 128]);
    let out = render(&img, &Document::default()).unwrap();
    let p = px(&out, 0, 0);
    assert_eq!(p[3], 128);
    for (a, b) in p[..3].iter().zip([200u8, 100, 50]) {
        assert!(a.abs_diff(b) <= 2, "{p:?}");
    }
}

#[test]
fn a_4k_image_with_many_annotations_renders() {
    let original = from_fn(3840, 2160, |x, y| ((x ^ y) & 0xff) as u8);
    let s = style(RED, 6.0, None);
    let doc = doc_with(vec![
        (
            Kind::Rectangle {
                rect: r(100.0, 100.0, 800.0, 400.0),
            },
            s.clone(),
        ),
        (
            Kind::Arrow {
                from: Point::new(1000.0, 1000.0),
                to: Point::new(2000.0, 1400.0),
            },
            s.clone(),
        ),
        (
            Kind::Text {
                at: Point::new(300.0, 700.0),
                text: "Step 1\nOpen settings".into(),
                size: 48.0,
                bold: true,
            },
            s.clone(),
        ),
        (
            Kind::Obscure {
                rect: r(2000.0, 200.0, 800.0, 500.0),
                mode: ObscureMode::Blur,
                strength: 30,
            },
            s.clone(),
        ),
        (
            Kind::Obscure {
                rect: r(200.0, 1200.0, 900.0, 600.0),
                mode: ObscureMode::Pixelate,
                strength: 24,
            },
            s,
        ),
    ]);
    let out = render(&original, &doc).unwrap();
    assert_eq!((out.width, out.height), (3840, 2160));
}

#[test]
fn an_obscure_region_partly_outside_the_image_still_redacts_the_part_inside() {
    let original = from_fn(40, 40, |x, y| if (x + y) % 2 == 0 { 0 } else { 255 });
    for mode in [ObscureMode::Pixelate, ObscureMode::Blur] {
        let doc = doc_with(vec![(
            Kind::Obscure {
                rect: r(-10.0, -10.0, 30.0, 30.0),
                mode,
                strength: 6,
            },
            Style::default(),
        )]);
        let out = render(&original, &doc).unwrap();
        let inside_untouched = (0..20u32)
            .flat_map(|y| (0..20u32).map(move |x| (x, y)))
            .filter(|&(x, y)| px(&out, x, y) == px(&original, x, y))
            .count();
        assert!(
            inside_untouched < 40,
            "{mode:?}: the checkerboard must be gone from the visible part, {inside_untouched} pixels survive"
        );
        assert_eq!(
            px(&out, 30, 30),
            px(&original, 30, 30),
            "{mode:?}: outside is untouched"
        );
    }
}

#[test]
fn extreme_obscure_strengths_still_destroy_the_pattern() {
    let original = from_fn(64, 64, |x, _| if x % 2 == 0 { 0 } else { 255 });
    for (mode, strength) in [
        (ObscureMode::Blur, 1_000_000),
        (ObscureMode::Blur, 0),
        (ObscureMode::Pixelate, 1_000_000),
        (ObscureMode::Pixelate, 0),
    ] {
        let doc = doc_with(vec![(
            Kind::Obscure {
                rect: r(8.0, 8.0, 48.0, 48.0),
                mode,
                strength,
            },
            Style::default(),
        )]);
        let out = render(&original, &doc).unwrap();
        let (min, max) = (16..48u32)
            .flat_map(|y| (16..48u32).map(move |x| (x, y)))
            .map(|(x, y)| px(&out, x, y)[0])
            .fold((255u8, 0u8), |(lo, hi), v| (lo.min(v), hi.max(v)));
        assert!(
            max - min < 200,
            "{mode:?} strength {strength}: the stripes are still fully visible ({min}..{max})"
        );
    }
}

#[test]
fn a_crop_over_an_obscured_area_never_shows_the_original_pixels() {
    let original = from_fn(64, 64, |x, _| if x % 2 == 0 { 0 } else { 255 });
    let mut e = Editor::new(Document::default());
    e.add(
        Kind::Obscure {
            rect: r(16.0, 16.0, 32.0, 32.0),
            mode: ObscureMode::Pixelate,
            strength: 32,
        },
        Style::default(),
    );
    e.set_crop(Some(PixelRect {
        x: 8,
        y: 8,
        w: 40,
        h: 40,
    }));
    let out = render(&original, e.document()).unwrap();
    assert_eq!((out.width, out.height), (40, 40));
    // Output pixel (x, y) is original (x + 8, y + 8): the obscured part is output 8..40.
    for y in 8..32u32 {
        for x in 8..32u32 {
            let v = px(&out, x, y)[0];
            assert!((i32::from(v) - 127).abs() <= 1, "({x},{y}) = {v}");
        }
    }
    assert_eq!(
        px(&out, 2, 2),
        px(&original, 10, 10),
        "outside the region the pixels are the original's"
    );
}
