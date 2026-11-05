use std::collections::HashMap;

use guide_core::{
    DescriptionPlacement, ImageAlign, ImageScaling, ItemId, Layout as LayoutKind, NewItem, Project,
    Rgba, StepLayout,
};

use super::*;

fn guide_with(n: usize) -> (Project, Vec<ItemId>, HashMap<ItemId, (u32, u32)>) {
    let mut g = Project::new_guide("My guide");
    g.guide_mut().unwrap().composition.margin = 20;
    g.guide_mut().unwrap().composition.spacing = 16;
    g.guide_mut().unwrap().composition.max_width = 600;
    let mut sizes = HashMap::new();
    let ids: Vec<ItemId> = (0..n)
        .map(|i| {
            let id = g.add_item(
                usize::MAX,
                NewItem {
                    title: format!("Step title {}", i + 1),
                    description: format!("Do thing {}", i + 1),
                    caption: String::new(),
                    image: format!("assets/s{i}.png"),
                },
            );
            sizes.insert(id, (1200, 800));
            id
        })
        .collect();
    (g, ids, sizes)
}

fn solid(w: u32, h: u32, c: [u8; 3]) -> Image {
    Image {
        width: w,
        height: h,
        rgba: [c[0], c[1], c[2], 255].repeat(w as usize * h as usize),
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

fn layout(g: &Project, sizes: &HashMap<ItemId, (u32, u32)>) -> ComposedLayout {
    compute_layout(g, sizes).unwrap()
}

fn image_of(l: &ComposedLayout, id: ItemId) -> RectF {
    l.images.iter().find(|i| i.step == id).unwrap().rect
}

fn box_of(l: &ComposedLayout, id: ItemId) -> RectF {
    l.boxes.iter().find(|b| b.step == id).unwrap().rect
}

#[test]
fn wrapping_keeps_every_word_and_every_line_fits() {
    let text = "The quick brown fox jumps over the lazy dog and keeps running through the forest until night falls";
    for width in [120.0, 200.0, 333.0, 800.0] {
        let lines = wrap_text(text, 20.0, width);
        assert!(lines.len() >= 2 || width >= 800.0, "{width}");
        for l in &lines {
            assert!(
                text::measure_text(l, 20.0).0 <= width,
                "line {l:?} is wider than {width}"
            );
        }
        let words: Vec<&str> = lines.iter().flat_map(|l| l.split_whitespace()).collect();
        assert_eq!(
            words,
            text.split_whitespace().collect::<Vec<_>>(),
            "no word lost or reordered at {width}"
        );
    }
    assert_eq!(wrap_text("short", 20.0, 500.0), vec!["short"]);
}

#[test]
fn wrapping_breaks_overlong_words_and_keeps_paragraphs() {
    let long = "x".repeat(200);
    let lines = wrap_text(&long, 20.0, 100.0);
    assert!(lines.len() > 2);
    assert!(lines.iter().all(|l| text::measure_text(l, 20.0).0 <= 100.0));
    assert_eq!(lines.concat(), long, "no character dropped");
    assert_eq!(
        wrap_text("one\n\ntwo\nthree", 20.0, 500.0),
        vec!["one", "", "two", "three"]
    );
    assert_eq!(
        wrap_text("tail\n\n\n", 20.0, 500.0),
        vec!["tail"],
        "trailing blank lines are trimmed"
    );
    assert!(wrap_text("   ", 20.0, 500.0).is_empty());
}

#[test]
fn steps_stack_in_order_with_the_configured_spacing_and_stay_inside_the_canvas() {
    let (g, ids, sizes) = guide_with(3);
    let l = layout(&g, &sizes);
    assert_eq!(l.width, 640, "content width plus two margins");
    for pair in ids.windows(2) {
        let (a, b) = (box_of(&l, pair[0]), box_of(&l, pair[1]));
        assert!(
            (b.y - (a.y + a.h) - 16.0).abs() < 0.01,
            "spacing between steps: {a:?} {b:?}"
        );
    }
    for id in &ids {
        let (bx, im) = (box_of(&l, *id), image_of(&l, *id));
        assert!(bx.x >= 20.0 && bx.right() <= 620.0 + 0.01, "{bx:?}");
        assert!(
            im.x >= bx.x - 0.01
                && im.right() <= bx.right() + 0.01
                && im.bottom() <= bx.bottom() + 0.01,
            "image inside its box"
        );
    }
    let last = box_of(&l, ids[2]);
    assert_eq!(
        l.height,
        (last.bottom() + 20.0).ceil() as u32,
        "bottom margin after the last step"
    );
    assert_eq!(
        l.boxes.iter().map(|b| b.number).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[test]
fn numbering_can_start_at_any_number() {
    let (g, ids, sizes) = guide_with(2);
    let l = compute_layout_from(&g, &sizes, 5).unwrap();
    assert_eq!(
        l.badges.iter().map(|b| b.number).collect::<Vec<_>>(),
        vec![5, 6]
    );
    assert_eq!(l.boxes.iter().map(|b| b.step).collect::<Vec<_>>(), ids);
    assert_eq!(
        compute_layout_from(&g, &sizes, 1).unwrap(),
        compute_layout(&g, &sizes).unwrap()
    );
    assert_eq!(
        compute_layout_from(&g, &sizes, 0).unwrap().badges[0].number,
        1,
        "0 is treated as 1"
    );
}

#[test]
fn layout_is_deterministic() {
    let (g, _, sizes) = guide_with(4);
    assert_eq!(layout(&g, &sizes), layout(&g, &sizes));
}

#[test]
fn excluded_steps_are_left_out_and_numbering_counts_only_included_ones() {
    let (mut g, ids, sizes) = guide_with(3);
    g.set_included(ids[1], false);
    let l = layout(&g, &sizes);
    assert_eq!(l.boxes.len(), 2);
    assert!(l.boxes.iter().all(|b| b.step != ids[1]));
    assert_eq!(
        l.badges.iter().map(|b| b.number).collect::<Vec<_>>(),
        vec![1, 2],
        "the third step is numbered 2"
    );
    assert!(l
        .texts
        .iter()
        .all(|t| !t.lines.join(" ").contains("Step title 2")));
}

#[test]
fn image_scaling_modes_keep_the_aspect_ratio() {
    let (mut g, ids, mut sizes) = guide_with(2);
    sizes.insert(ids[0], (1200, 800)); // wider than the 600 slot
    sizes.insert(ids[1], (300, 200)); // narrower
    let l = layout(&g, &sizes);
    let (big, small) = (image_of(&l, ids[0]), image_of(&l, ids[1]));
    assert_eq!((big.w, big.h), (600.0, 400.0), "shrunk to the slot");
    assert_eq!(
        (small.w, small.h),
        (300.0, 200.0),
        "small images are not enlarged"
    );

    g.guide_mut().unwrap().composition.image_scaling = ImageScaling::FillWidth;
    let l = layout(&g, &sizes);
    let small = image_of(&l, ids[1]);
    assert_eq!(
        (small.w, small.h),
        (600.0, 400.0),
        "enlarged to the slot width"
    );
    for id in &ids {
        let r = image_of(&l, *id);
        let (iw, ih) = sizes[id];
        assert!(
            ((r.w / r.h) - (iw as f32 / ih as f32)).abs() < 0.01,
            "aspect ratio kept"
        );
    }
}

#[test]
fn a_step_can_limit_its_own_image_width() {
    let (mut g, ids, sizes) = guide_with(1);
    g.guide_mut().unwrap().step_layouts.insert(
        ids[0],
        StepLayout {
            max_width: Some(300),
            show_description: None,
        },
    );
    let r = image_of(&layout(&g, &sizes), ids[0]);
    assert_eq!((r.w, r.h), (300.0, 200.0));
}

#[test]
fn alignment_places_narrow_images_left_centre_or_right() {
    let (mut g, ids, mut sizes) = guide_with(1);
    sizes.insert(ids[0], (200, 100));
    let slot_x = 20.0;
    for (align, expected_x) in [
        (ImageAlign::Left, slot_x),
        (ImageAlign::Center, slot_x + 200.0),
        (ImageAlign::Right, slot_x + 400.0),
    ] {
        g.guide_mut().unwrap().composition.image_align = align;
        assert_eq!(
            image_of(&layout(&g, &sizes), ids[0]).x,
            expected_x,
            "{align:?}"
        );
    }
}

#[test]
fn description_goes_above_or_below_the_image() {
    let (mut g, ids, sizes) = guide_with(1);
    let desc_y = |l: &ComposedLayout| {
        l.texts
            .iter()
            .find(|t| t.lines[0].starts_with("Do thing"))
            .unwrap()
            .y
    };
    let above = layout(&g, &sizes);
    assert!(desc_y(&above) < image_of(&above, ids[0]).y);
    g.guide_mut().unwrap().composition.description_placement = DescriptionPlacement::BelowImage;
    let below = layout(&g, &sizes);
    assert!(
        desc_y(&below) >= image_of(&below, ids[0]).bottom(),
        "description is under the image"
    );
    assert_eq!(
        above.height, below.height,
        "moving the description does not change the total height"
    );
}

#[test]
fn visibility_toggles_remove_items_and_never_add_height() {
    let (mut g, _, sizes) = guide_with(2);
    g.guide_mut().unwrap().introduction = "An introduction".into();
    g.guide_mut().unwrap().final_notes = "Thanks for reading".into();
    let full = layout(&g, &sizes);
    let has = |l: &ComposedLayout, s: &str| l.texts.iter().any(|t| t.lines.join(" ").contains(s));
    assert!(
        has(&full, "My guide")
            && has(&full, "An introduction")
            && has(&full, "Thanks")
            && has(&full, "Do thing 1")
    );
    assert_eq!(full.badges.len(), 2);

    g.guide_mut().unwrap().composition.show_numbers = false;
    assert!(layout(&g, &sizes).badges.is_empty());
    g.guide_mut().unwrap().composition.show_titles = false;
    assert!(!has(&layout(&g, &sizes), "Step title"));
    g.guide_mut().unwrap().composition.show_descriptions = false;
    assert!(!has(&layout(&g, &sizes), "Do thing"));
    g.guide_mut().unwrap().composition.show_guide_title = false;
    g.guide_mut().unwrap().composition.show_introduction = false;
    g.guide_mut().unwrap().composition.show_final_notes = false;
    let bare = layout(&g, &sizes);
    assert!(bare.texts.is_empty());
    assert!(bare.height < full.height);
}

#[test]
fn a_step_can_hide_just_its_own_description() {
    let (mut g, ids, sizes) = guide_with(2);
    g.guide_mut().unwrap().step_layouts.insert(
        ids[0],
        StepLayout {
            max_width: None,
            show_description: Some(false),
        },
    );
    let l = layout(&g, &sizes);
    let has = |s: &str| l.texts.iter().any(|t| t.lines.join(" ").contains(s));
    assert!(!has("Do thing 1") && has("Do thing 2"));
}

#[test]
fn long_text_wraps_inside_the_content_width_and_never_truncates() {
    let (mut g, ids, sizes) = guide_with(1);
    let words = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango";
    g.item_mut(ids[0]).unwrap().description = words.repeat(3);
    let l = layout(&g, &sizes);
    let t = l
        .texts
        .iter()
        .find(|t| t.lines[0].starts_with("alpha"))
        .unwrap();
    assert!(t.lines.len() > 3);
    assert!(t
        .lines
        .iter()
        .all(|line| text::measure_text(line, t.size).0 <= 600.0));
    let got: Vec<&str> = t.lines.iter().flat_map(|l| l.split_whitespace()).collect();
    assert_eq!(got, words.repeat(3).split_whitespace().collect::<Vec<_>>());
    assert!(box_of(&l, ids[0]).bottom() <= l.height as f32);
}

#[test]
fn a_grid_places_steps_side_by_side_and_rows_share_a_height() {
    let (mut g, ids, mut sizes) = guide_with(3);
    g.guide_mut().unwrap().composition.layout = LayoutKind::Grid { columns: 2 };
    sizes.insert(ids[0], (600, 600)); // taller than its neighbour
    sizes.insert(ids[1], (600, 200));
    let l = layout(&g, &sizes);
    let cell = ((600.0 - 16.0) / 2.0_f32).floor();
    let (a, b, c) = (box_of(&l, ids[0]), box_of(&l, ids[1]), box_of(&l, ids[2]));
    assert_eq!(a.y, b.y, "first row");
    assert_eq!((a.w, b.w), (cell, cell));
    assert_eq!(b.x, a.x + cell + 16.0);
    assert!(
        (c.y - (a.y + a.h.max(b.h) + 16.0)).abs() < 0.01,
        "second row starts below the taller cell: {a:?} {b:?} {c:?}"
    );
    assert_eq!(c.x, a.x, "third step wraps to the first column");
    assert!(image_of(&l, ids[0]).w <= cell);
}

#[test]
fn an_unknown_screenshot_size_gets_a_placeholder_instead_of_failing() {
    let (g, ids, _) = guide_with(1);
    let l = layout(&g, &HashMap::new());
    let r = image_of(&l, ids[0]);
    assert!(r.w > 100.0 && r.h > 50.0);
}

#[test]
fn oversized_results_are_refused_before_anything_is_drawn() {
    let (mut g, _, mut sizes) = guide_with(0);
    g.guide_mut().unwrap().composition.max_width = 10_000;
    g.guide_mut().unwrap().composition.image_scaling = ImageScaling::FillWidth;
    for _ in 0..60 {
        let id = g.add_item(
            usize::MAX,
            NewItem {
                title: "t".into(),
                description: String::new(),
                caption: String::new(),
                image: "assets/x.png".into(),
            },
        );
        sizes.insert(id, (4000, 4000));
    }
    let err = compute_layout(&g, &sizes).unwrap_err();
    assert!(matches!(err, ComposeError::TooLarge(_, _)), "{err}");
    let msg = err.to_string();
    assert!(
        msg.contains("separate images") && msg.contains("narrower"),
        "the message offers alternatives: {msg}"
    );
}

#[test]
fn an_empty_guide_is_just_margins_and_a_title() {
    let (mut g, _, sizes) = guide_with(0);
    g.guide_mut().unwrap().composition.show_guide_title = false;
    let l = layout(&g, &sizes);
    assert_eq!((l.width, l.height), (640, 40));
    assert!(l.boxes.is_empty());
}

#[test]
fn rendering_matches_the_layout_size_and_draws_each_part() {
    let (g, ids, sizes) = guide_with(2);
    let l = layout(&g, &sizes);
    let colours = [[200u8, 30, 30], [30, 30, 200]];
    let mut ask = |id: ItemId| {
        Some(solid(
            1200,
            800,
            colours[ids.iter().position(|i| *i == id).unwrap()],
        ))
    };
    let img = render_layout(&l, 1.0, &mut ask).unwrap();
    assert_eq!((img.width, img.height), (l.width, l.height));
    assert_eq!(
        px(&img, 2, 2),
        [255, 255, 255, 255],
        "background in the margin"
    );
    for (id, colour) in ids.iter().zip(colours) {
        let r = image_of(&l, *id);
        let p = px(&img, (r.x + r.w / 2.0) as u32, (r.y + r.h / 2.0) as u32);
        assert_eq!(
            &p[..3],
            &colour,
            "the step's screenshot is drawn in its rectangle"
        );
    }
    let b = &l.badges[0];
    assert_eq!(
        px(&img, (b.cx + b.radius * 0.8) as u32, b.cy as u32)[..3],
        [40, 110, 200],
        "the number badge"
    );
    let ink = img
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] < 100 && p[1] < 100 && p[2] < 100 && p[0] == p[1])
        .count();
    assert!(ink > 100, "text was drawn");
}

#[test]
fn the_preview_scale_uses_the_same_layout_as_the_final_size() {
    let (g, ids, sizes) = guide_with(2);
    let l = layout(&g, &sizes);
    let mut ask = |_: ItemId| Some(solid(1200, 800, [10, 160, 10]));
    let full = render_layout(&l, 1.0, &mut ask).unwrap();
    let half = render_layout(&l, 0.5, &mut ask).unwrap();
    assert_eq!(
        (half.width, half.height),
        (
            (l.width as f32 * 0.5).ceil() as u32,
            (l.height as f32 * 0.5).ceil() as u32
        )
    );
    for id in &ids {
        let r = image_of(&l, *id);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        assert_eq!(
            px(&full, cx as u32, cy as u32),
            px(&half, (cx * 0.5) as u32, (cy * 0.5) as u32),
            "the same thing is at the same relative place"
        );
        // Just outside the image on each side is background in both.
        for (scale, im) in [(1.0, &full), (0.5, &half)] {
            let above = ((r.y - 4.0) * scale) as u32;
            assert_eq!(px(im, ((r.x + r.w / 2.0) * scale) as u32, above)[3], 255);
        }
    }
    let wrong_scale_text = |im: &Image| {
        im.rgba
            .chunks_exact(4)
            .filter(|p| p[0] < 100 && p[0] == p[1])
            .count()
    };
    assert!(
        wrong_scale_text(&half) > 20 && wrong_scale_text(&half) < wrong_scale_text(&full),
        "text is drawn smaller, not dropped"
    );
}

#[test]
fn a_missing_screenshot_is_drawn_as_a_labelled_placeholder() {
    let (g, ids, sizes) = guide_with(1);
    let l = layout(&g, &sizes);
    let img = render_layout(&l, 1.0, &mut |_| None).unwrap();
    let r = image_of(&l, ids[0]);
    let p = px(&img, (r.x + 8.0) as u32, (r.y + r.h - 8.0) as u32);
    assert_eq!(
        &p[..3],
        &[225, 225, 225],
        "grey placeholder, not the white background"
    );
}

#[test]
fn drawing_stops_at_once_when_asked_instead_of_placeholdering_the_rest() {
    let (g, _, sizes) = guide_with(4);
    let l = layout(&g, &sizes);
    let asked = std::cell::Cell::new(0);
    let mut images = |_: ItemId| {
        asked.set(asked.get() + 1);
        Some(solid(10, 10, [0, 0, 0]))
    };
    // Stop once two screenshots have been asked for.
    let err = render_layout_until(&l, 1.0, &mut images, &|| asked.get() >= 2).unwrap_err();
    assert!(matches!(err, ComposeError::Cancelled), "{err}");
    assert_eq!(
        asked.get(),
        2,
        "the remaining screenshots were never asked for"
    );
    // Never stopping is the same as render_layout.
    let mut images = |_: ItemId| Some(solid(10, 10, [0, 0, 0]));
    let a = render_layout_until(&l, 1.0, &mut images, &|| false).unwrap();
    let mut images = |_: ItemId| Some(solid(10, 10, [0, 0, 0]));
    assert_eq!(a.rgba, render_layout(&l, 1.0, &mut images).unwrap().rgba);
}

#[test]
fn the_placeholder_says_what_the_caller_asked_it_to_say() {
    let (g, _, sizes) = guide_with(1);
    let mut l = layout(&g, &sizes);
    assert_eq!(l.missing_label, DEFAULT_MISSING_LABEL);
    let english = render_layout(&l, 1.0, &mut |_| None).unwrap();
    l.missing_label = "Captura no encontrada".into();
    let spanish = render_layout(&l, 1.0, &mut |_| None).unwrap();
    l.missing_label = String::new();
    let blank = render_layout(&l, 1.0, &mut |_| None).unwrap();
    assert_ne!(
        english.rgba, spanish.rgba,
        "the label is drawn into the image"
    );
    assert_ne!(spanish.rgba, blank.rgba);
}

#[test]
fn the_background_colour_and_scale_limits_are_respected() {
    let (mut g, _, sizes) = guide_with(1);
    g.guide_mut().unwrap().composition.background = Rgba([10, 20, 30, 255]);
    let l = layout(&g, &sizes);
    let img = render_layout(&l, 1.0, &mut |_| Some(solid(10, 10, [0, 0, 0]))).unwrap();
    assert_eq!(px(&img, 1, 1), [10, 20, 30, 255]);
    // An absurd scale is clamped instead of allocating something huge.
    let tiny = render_layout(&l, 0.0, &mut |_| None).unwrap();
    assert!(tiny.width >= 1 && tiny.width < l.width / 50);
}

#[test]
fn cropped_size_uses_the_crop_when_present() {
    assert_eq!(cropped_size((100, 80), None), (100, 80));
    assert_eq!(
        cropped_size(
            (100, 80),
            Some(PixelRect {
                x: 5,
                y: 5,
                w: 30,
                h: 20
            })
        ),
        (30, 20)
    );
}

fn diff_with(
    layout: guide_core::DiffLayout,
) -> (Project, Vec<ItemId>, HashMap<ItemId, (u32, u32)>) {
    let mut d = Project::new_diff("Login page");
    {
        let s = d.diff_mut().unwrap();
        s.layout = layout;
        s.margin = 20;
        s.spacing = 16;
    }
    let mut sizes = HashMap::new();
    let ids: Vec<ItemId> = ["Before", "After"]
        .iter()
        .enumerate()
        .map(|(i, cap)| {
            let id = d.add_item(
                usize::MAX,
                NewItem {
                    title: (*cap).into(),
                    description: String::new(),
                    caption: (*cap).into(),
                    image: format!("assets/s{i}.png"),
                },
            );
            sizes.insert(id, (400, 300));
            id
        })
        .collect();
    (d, ids, sizes)
}

#[test]
fn a_horizontal_diff_puts_the_screenshots_side_by_side_at_one_height() {
    let (d, ids, sizes) = diff_with(guide_core::DiffLayout::Horizontal);
    let l = compute_diff_layout(&d, &sizes).unwrap();
    assert_eq!(l.images.len(), 2);
    let (a, b) = (&l.images[0], &l.images[1]);
    assert_eq!((a.step, b.step), (ids[0], ids[1]));
    assert_eq!(a.rect.y, b.rect.y);
    assert_eq!(a.rect.h, b.rect.h);
    assert_eq!(b.rect.x, a.rect.x + a.rect.w + 16.0);
    assert_eq!(l.width, (20.0 * 2.0 + 400.0 * 2.0 + 16.0) as u32);
}

#[test]
fn a_vertical_diff_stacks_the_screenshots_at_one_width() {
    let (d, _, sizes) = diff_with(guide_core::DiffLayout::Vertical);
    let l = compute_diff_layout(&d, &sizes).unwrap();
    let (a, b) = (&l.images[0], &l.images[1]);
    assert_eq!(a.rect.x, b.rect.x);
    assert_eq!(a.rect.w, b.rect.w);
    assert!(b.rect.y >= a.rect.y + a.rect.h + 16.0);
}

#[test]
fn diff_header_and_captions_are_optional() {
    let (mut d, _, sizes) = diff_with(guide_core::DiffLayout::Horizontal);
    d.diff_mut().unwrap().description = "What changed".into();
    let with_all = compute_diff_layout(&d, &sizes).unwrap();
    let texts: Vec<String> = with_all
        .texts
        .iter()
        .flat_map(|t| t.lines.clone())
        .collect();
    assert!(texts.contains(&"Login page".to_string()), "{texts:?}");
    assert!(texts.contains(&"What changed".to_string()));
    assert!(texts.contains(&"Before".to_string()) && texts.contains(&"After".to_string()));

    d.diff_mut().unwrap().show_header = false;
    d.diff_mut().unwrap().show_captions = false;
    let bare = compute_diff_layout(&d, &sizes).unwrap();
    assert!(bare.texts.is_empty());
    assert!(bare.height < with_all.height);
}
