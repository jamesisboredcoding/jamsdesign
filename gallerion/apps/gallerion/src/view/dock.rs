//! Right-hand panel dock: Color, Properties, Layers.

use iced::alignment::Vertical;
use iced::widget::{button, column, container, image, mouse_area, pick_list, row, scrollable, sensor, slider, space, text, text_input, tooltip};
use iced::{Color, Element, Fill, Length, Padding, Shrink};
use jd_core::Hsv;
use jd_raster::BlendMode;
use jd_ui::widget::{bold, hline, hover_tip, icon_button, label, layer_row, panel_header, panel_header_grip};
use jd_ui::{Icon, icon, style, tokens::*};

use super::{color, to_color};
use crate::app::{DropTarget, Message, PANEL_SPLIT, Panel, Photo, RENAME_INPUT};

pub fn view(app: &Photo) -> Element<'_, Message> {
    // Window menu toggles which panels show; the bottom one takes the remaining height.
    // Drag the strip below a panel to resize it, or its tab strip to move it.
    let shown = app.shown_panels();
    if shown.is_empty() {
        // Minimal workspace: no dock at all.
        return space().into();
    }
    let drop_line = |slot: usize| {
        let on = app.panel_drop == Some(slot);
        container(space()).width(Fill).height(2).style(style::fill(if on { ACCENT_PRIMARY } else { iced::Color::TRANSPARENT }))
    };
    let mut col = column![drop_line(0)].height(Fill);
    for (i, &p) in shown.iter().enumerate() {
        let last = i + 1 == shown.len();
        let height = if last {
            Length::Fill
        } else {
            match (app.panel_heights.get(&p), p) {
                (Some(h), _) => Length::Fixed(*h),
                // Layers lists fill whatever they get; give it a sensible size when it isn't last.
                (None, Panel::Layers) => Length::Fixed(300.0),
                (None, _) => Shrink,
            }
        };
        let body = match p {
            Panel::Color => color_panel(app),
            Panel::Properties => properties_panel(app),
            Panel::Layers => layers_panel(app),
        };
        let body = container(body).width(Fill).height(height).clip(true);
        col = col.push(sensor(body).on_show(move |s| Message::PanelMeasured(p, s.height)).on_resize(move |s| Message::PanelMeasured(p, s.height)));
        if !last {
            let resizing = matches!(app.ui_drag, Some(crate::app::UiDrag::PanelSplit { panel, .. }) if panel == p);
            col = col.push(
                mouse_area(container(hline(if resizing { ACCENT_PRIMARY } else { BORDER_DEFAULT })).height(PANEL_SPLIT - 2.0).center_y(PANEL_SPLIT - 2.0))
                    .on_press(Message::PanelResizeStart(p))
                    .interaction(iced::mouse::Interaction::ResizingVertically),
            );
        }
        col = col.push(drop_line(i + 1));
    }
    // Tracks the pointer so the layer right-click menu opens where you clicked.
    mouse_area(container(col).width(app.dock_width).height(Fill).style(style::fill(BG_PANEL)))
        .on_move(Message::DockPointer)
        .into()
}

/// A dock panel: its tab strip (drag the empty part to move the panel; ≡ for its menu) and
/// padded content.
fn framed<'a>(
    p: Panel,
    tabs: &[&'a str],
    active: usize,
    on_tab: impl Fn(usize) -> Message,
    content: impl Into<Element<'a, Message>>,
    height: Length,
) -> Element<'a, Message> {
    column![
        panel_header_grip(tabs, active, on_tab, Message::PanelGrab(p), Message::OpenPanelMenu(p)),
        container(content).width(Fill).height(Fill).padding(10),
    ]
    .height(height)
    .into()
}

fn color_panel(app: &Photo) -> Element<'_, Message> {
    let content: Element<'_, Message> = match app.color_tab {
        0 => {
            let [r, g, b] = app.fg.to_rgba().to_srgb8();
            let hex = app.fg.to_rgba().to_hex();
            let rgb = [r, g, b];
            let set = |c: [u8; 3]| Message::SetFg(Hsv::from_rgba(jd_core::Rgba::from_srgb8(c[0], c[1], c[2])));
            let channel = move |name: &'static str, key: &'static str, i: usize| {
                row![
                    text(name).size(10).color(TEXT_MUTED),
                    super::field::number_live(app, key, rgb[i].to_string(), Fill, 1.0, 0.0..=255.0, move |v| {
                        let mut c = rgb;
                        c[i] = v.round() as u8;
                        set(c)
                    }),
                ]
                .spacing(4)
                .align_y(Vertical::Center)
            };
            let hex_field = row![
                text("#").size(10).color(TEXT_MUTED),
                super::field::custom_live(app, "hex", hex.trim_start_matches('#').to_owned(), Fill, |t| {
                    jd_core::Rgba::from_hex(t.trim().trim_start_matches('#')).map(|c| Message::SetFg(Hsv::from_rgba(c)))
                }),
            ]
            .spacing(4)
            .align_y(Vertical::Center);
            column![
                row![color::sv_square(app.fg), color::hue_strip(app.fg)].spacing(8),
                row![
                    container(hex_field).width(Length::FillPortion(2)),
                    container(channel("R", "red", 0)).width(Length::FillPortion(1)),
                    container(channel("G", "green", 1)).width(Length::FillPortion(1)),
                    container(channel("B", "blue", 2)).width(Length::FillPortion(1)),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into()
        }
        _ => swatches(),
    };
    framed(Panel::Color, &["Color", "Swatches"], app.color_tab, Message::ColorTab, content, Shrink)
}

fn swatches<'a>() -> Element<'a, Message> {
    const HEXES: &[&str] = &[
        "000000", "404040", "808080", "BFBFBF", "FFFFFF", "E5594D", "F2733F", "F2B13F", "F2E33F",
        "8FD14F", "4CC38A", "3FC6D9", "4C8DFF", "6E5BF2", "B04CF2", "F24CB5", "7A3E8E", "1B2A5C",
        "5B3A7A", "FFC27A",
    ];
    let mut grid = row![].spacing(4);
    let mut rows = column![].spacing(4);
    for (i, h) in HEXES.iter().enumerate() {
        let hsv = Hsv::from_rgba(jd_core::Rgba::from_hex(h).expect("valid swatch hex"));
        let c = to_color(hsv);
        grid = grid.push(
            button(space())
                .width(22)
                .height(22)
                .style(move |_, status| button::Style {
                    background: Some(c.into()),
                    border: iced::Border {
                        color: if matches!(status, button::Status::Hovered) { Color::WHITE } else { BORDER_SUBTLE },
                        width: 1.0,
                        radius: 2.0.into(),
                    },
                    ..Default::default()
                })
                .on_press(Message::SetFg(hsv)),
        );
        if (i + 1) % 10 == 0 {
            rows = rows.push(std::mem::replace(&mut grid, row![].spacing(4)));
        }
    }
    rows.push(grid).into()
}

fn properties_panel(app: &Photo) -> Element<'_, Message> {
    let content: Element<'_, Message> = match (app.props_tab, app.document().and_then(|d| d.active())) {
        (0, Some(layer)) if layer.adjustment.is_some() => {
            let name = layer.adjustment.as_ref().map_or("", |a| a.name());
            column![
                row![icon(Icon::Adjust, 14.0, ICON), text(&layer.name).size(TEXT_SIZE_SMALL).font(bold())].spacing(6).align_y(Vertical::Center),
                label(format!("{name} adjustment layer: it changes every layer below it. Paint its mask to limit where.")),
                button(text("Edit Adjustment...").size(TEXT_SIZE_SMALL))
                    .padding(Padding::from([5, 10]))
                    .style(style::ghost(false))
                    .on_press_maybe(app.adjusting.is_none().then_some(Message::EditAdjustment(layer.id))),
                row![
                    label("Opacity").width(Fill),
                    super::field::number(app, "props-opacity", format!("{:.0}%", layer.opacity * 100.0), 52, 100.0, 0.0..=1.0, Message::LayerOpacity),
                ]
                .align_y(Vertical::Center),
                slider(0.0..=1.0, layer.opacity, Message::LayerOpacity).step(0.01_f32).on_release(Message::EndGesture).style(style::slider),
            ]
            .spacing(8)
            .into()
        }
        (0, Some(layer)) => column![
            row![icon(Icon::Layers, 14.0, ICON), text(&layer.name).size(TEXT_SIZE_SMALL).font(bold())]
                .spacing(6)
                .align_y(Vertical::Center),
            row![
                label("Opacity").width(Fill),
                super::field::number(app, "props-opacity", format!("{:.0}%", layer.opacity * 100.0), 52, 100.0, 0.0..=1.0, Message::LayerOpacity),
            ]
            .align_y(Vertical::Center),
            slider(0.0..=1.0, layer.opacity, Message::LayerOpacity)
                .step(0.01_f32)
                .on_release(Message::EndGesture)
                .style(style::slider),
            row![
                label("Blend").width(Fill),
                blend_picker(Some(layer)),
            ]
            .align_y(Vertical::Center),
        ]
        .spacing(8)
        .into(),
        (0, None) => label("No layer selected").into(),
        _ => adjustments(app),
    };
    framed(Panel::Properties, &["Properties", "Adjustments"], app.props_tab, Message::PropsTab, content, Shrink)
}

/// The Adjustments tab: every adjustment and filter one click away (they apply to the active
/// layer, like the Image and Filter menus; adjustment layers aren't there yet).
/// Photoshop's Adjustments panel: adjustment layers (non-destructive), plus the filters, which
/// apply to the active layer.
fn adjustments(app: &Photo) -> Element<'_, Message> {
    use crate::adjust::OpKind::*;
    let doc = app.document().is_some() && app.adjusting.is_none();
    let layers = [
        ("Brightness/Contrast", BrightnessContrast),
        ("Levels", Levels),
        ("Curves", Curves),
        ("Hue/Saturation", HueSaturation),
        ("Threshold", Threshold),
        ("Posterize", Posterize),
        ("Invert", Invert),
        ("Desaturate", Desaturate),
    ];
    let mut adjust_grid = column![].spacing(4);
    for pair in layers.chunks(2) {
        let mut r = row![].spacing(4);
        for &(name, kind) in pair {
            r = r.push(
                button(text(name).size(TEXT_SIZE_SMALL))
                    .width(Fill)
                    .padding(Padding::from([5, 8]))
                    .style(style::ghost(false))
                    .on_press_maybe(doc.then_some(Message::NewAdjustmentLayer(kind))),
            );
        }
        adjust_grid = adjust_grid.push(r);
    }
    let editable = app.document().and_then(|d| d.active()).is_some_and(|l| !l.locked && l.adjustment.is_none());
    let items = [
        ("Gaussian Blur", GaussianBlur),
        ("Box Blur", BoxBlur),
        ("Sharpen", Sharpen),
        ("Unsharp Mask", UnsharpMask),
        ("High Pass", HighPass),
        ("Add Noise", AddNoise),
        ("Mosaic", Mosaic),
    ];
    let mut grid = column![].spacing(4);
    for pair in items.chunks(2) {
        let mut r = row![].spacing(4);
        for &(name, kind) in pair {
            r = r.push(
                button(text(name).size(TEXT_SIZE_SMALL))
                    .width(Fill)
                    .padding(Padding::from([5, 8]))
                    .style(style::ghost(false))
                    .on_press_maybe(editable.then_some(Message::OpenOp(kind))),
            );
        }
        grid = grid.push(r);
    }
    column![
        label("Add an adjustment layer (changes everything below it, editable later):"),
        adjust_grid,
        label(if editable { "Filters (applied to the active layer):" } else { "Filters (select an unlocked pixel layer):" }),
        grid,
    ]
    .spacing(8)
    .into()
}

/// The Channels tab: the composite image's channels. Clicking one loads it as a selection
/// (bright = selected), like Ctrl+clicking a channel in Photoshop.
/// Paths: the document's vector paths (drawn with the Pen tool). The selected one shows on
/// the canvas; the buttons fill, stroke or select with it.
fn paths_panel(app: &Photo) -> Element<'_, Message> {
    let Some(doc) = app.document() else { return label("No document open.").into() };
    let mut list = column![].spacing(2);
    for (i, p) in doc.paths.iter().enumerate() {
        let selected = doc.active_path == Some(i);
        let points: usize = p.subpaths.iter().map(|s| s.anchors.len()).sum();
        list = list.push(
            button(
                row![
                    icon(Icon::Pen, 14.0, ICON),
                    text(&p.name).size(TEXT_SIZE).width(Fill),
                    text(format!("{points} pts")).size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
                ]
                .spacing(8)
                .align_y(Vertical::Center),
            )
            .width(Fill)
            .padding(Padding::from([6, 8]))
            .style(style::row(selected))
            .on_press(Message::SelectPath(Some(i))),
        );
    }
    if doc.paths.is_empty() {
        list = list.push(label("No paths yet: draw one with the Pen tool (P)."));
    }
    let has = doc.active_path.and_then(|i| doc.paths.get(i)).is_some_and(|p| !p.is_empty());
    let mut actions = row![space::horizontal()].spacing(6).align_y(Vertical::Center);
    for (ic, msg, tip) in [
        (Icon::Shape, has.then_some(Message::FillPath), "Fill path with the foreground colour"),
        (Icon::Brush, has.then_some(Message::StrokePath), "Stroke path with the brush"),
        (Icon::Marquee, has.then_some(Message::PathToSelection), "Load path as a selection (Ctrl+Enter)"),
        (Icon::Plus, Some(Message::NewPath), "New path"),
        (Icon::Trash, doc.active_path.map(|_| Message::DeletePath), "Delete path"),
    ] {
        actions = actions.push(hover_tip(icon_button(ic, 14.0, false, msg), tip, tooltip::Position::Top));
    }
    column![
        // Clicking the empty part of the list deselects the path (hides it), like Photoshop.
        mouse_area(scrollable(list).height(Fill)).on_press(Message::SelectPath(None)),
        container(actions).padding(Padding::from([4, 8])).style(style::fill(BG_PANEL_HEADER)),
    ]
    .spacing(6)
    .into()
}

fn channels_panel(app: &Photo) -> Element<'_, Message> {
    let Some(open) = app.doc() else { return label("No document open.").into() };
    let names = ["RGB", "Red", "Green", "Blue"];
    let mut list = column![];
    for (i, name) in names.iter().enumerate() {
        let thumb: Element<'_, Message> = match open.channel_thumbs.get(i) {
            Some(h) => image(h.clone()).width(40).height(28).content_fit(iced::ContentFit::Contain).into(),
            None => space().width(40).into(),
        };
        let row = row![
            container(thumb).width(40).height(28).center_x(40).center_y(28).style(|_| container::Style {
                background: Some(iced::Color::from_rgb8(0xD8, 0xD8, 0xD8).into()),
                border: iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: 2.0.into() },
                ..Default::default()
            }),
            text(*name).size(TEXT_SIZE).width(Fill),
        ]
        .spacing(10)
        .align_y(Vertical::Center);
        list = list.push(
            tooltip(
                button(row).width(Fill).height(40).padding(Padding::from([0, 10])).style(style::row(false)).on_press(Message::LoadChannel(i as u8)),
                container(text("Load as selection").size(TEXT_SIZE_SMALL)).padding(6).style(style::input),
                tooltip::Position::Left,
            ),
        );
        list = list.push(hline(BORDER_DEFAULT));
    }
    column![list, container(label("Click a channel to select by its brightness.")).padding(10)].into()
}

/// A blend mode choice; groups also offer Pass Through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlendChoice {
    PassThrough,
    Mode(BlendMode),
}

impl std::fmt::Display for BlendChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlendChoice::PassThrough => f.write_str("Pass Through"),
            BlendChoice::Mode(m) => m.fmt(f),
        }
    }
}

fn blend_picker<'a>(layer: Option<&jd_raster::Layer>) -> Element<'a, Message> {
    let group = layer.and_then(|l| l.group);
    let mut options: Vec<BlendChoice> = Vec::with_capacity(BlendMode::ALL.len() + 1);
    if group.is_some() {
        options.push(BlendChoice::PassThrough);
    }
    options.extend(BlendMode::ALL.iter().map(|m| BlendChoice::Mode(*m)));
    let current = match (group, layer) {
        (Some(g), _) if g.pass_through => BlendChoice::PassThrough,
        (_, Some(l)) => BlendChoice::Mode(l.blend),
        _ => BlendChoice::Mode(BlendMode::Normal),
    };
    pick_list(options, Some(current), |c| match c {
        BlendChoice::PassThrough => Message::GroupPassThrough,
        BlendChoice::Mode(m) => Message::LayerBlend(m),
    })
    .text_size(TEXT_SIZE_SMALL)
    .padding(Padding::from([3, 6]))
    .width(140)
    .style(style::pick_list)
    .into()
}

/// Half a layer row's height (2px slot line + 40px row + 1px divider).
const ROW_HALF: f32 = 21.5;

fn layers_panel(app: &Photo) -> Element<'_, Message> {
    if app.layers_tab != 0 {
        let body = if app.layers_tab == 2 { paths_panel(app) } else { channels_panel(app) };
        return framed(Panel::Layers, &["Layers", "Channels", "Paths"], app.layers_tab, Message::LayersTab, body, Fill);
    }

    let Some(doc) = app.document() else {
        return framed(Panel::Layers, &["Layers", "Channels", "Paths"], app.layers_tab, Message::LayersTab, label("No document open."), Fill);
    };
    let active = doc.active();
    let controls = row![
        blend_picker(active),
        space::horizontal(),
        label("Opacity:"),
        super::field::number(app, "layer-opacity", format!("{:.0}%", active.map_or(100.0, |l| l.opacity * 100.0)), 52, 100.0, 0.0..=1.0, Message::LayerOpacity),
    ]
    .spacing(8)
    .align_y(Vertical::Center);

    // Pixel/position locks come with painting and the Move tool; "lock all" works now.
    let locked = active.is_some_and(|l| l.locked);
    let lock_all = tooltip(
        icon_button(Icon::Lock, 13.0, locked, Some(Message::ToggleLock)).padding(3),
        container(text(if locked { "Unlock layer" } else { "Lock layer" }).size(TEXT_SIZE_SMALL))
            .padding(6)
            .style(style::input),
        tooltip::Position::Top,
    );
    let (lock_px, lock_pos) = active.map_or((false, false), |l| (l.lock_pixels, l.lock_position));
    let locks = row![
        label("Lock:"),
        hover_tip(
            icon_button(Icon::Brush, 13.0, lock_px, Some(Message::ToggleLockPixels)).padding(3),
            if lock_px { "Unlock image pixels" } else { "Lock image pixels (no painting or filters)" },
            tooltip::Position::Top,
        ),
        hover_tip(
            icon_button(Icon::Move, 13.0, lock_pos, Some(Message::ToggleLockPosition)).padding(3),
            if lock_pos { "Unlock position" } else { "Lock position (no moving or transforming)" },
            tooltip::Position::Top,
        ),
        lock_all,
    ]
    .spacing(4)
    .align_y(Vertical::Center);

    let open = app.doc().expect("document() implies doc()");
    let active_id = doc.active_id();
    let mut list = column![];
    // Rows shown, top of the stack first (like every layers panel), skipping what's inside
    // collapsed groups. Drop targets: the upper half of a row means "above it"; the lower
    // half "below it", which for an open group is the top of its contents.
    let layers = doc.layers();
    let collapsed = |l: &jd_raster::Layer| {
        let mut cur = l.parent;
        while let Some(p) = cur {
            match doc.layer(p) {
                Some(g) if g.group.is_some_and(|g| !g.open) => return true,
                Some(g) => cur = g.parent,
                None => return false,
            }
        }
        false
    };
    let rows: Vec<(usize, &jd_raster::Layer)> = layers.iter().enumerate().rev().filter(|(_, l)| !collapsed(l)).collect();
    let upper = |i: usize| DropTarget::Slot(i + 1, layers[i].parent);
    let lower = |i: usize| match layers[i].group {
        Some(g) if g.open => DropTarget::Slot(i, Some(layers[i].id)),
        _ => DropTarget::Slot(*doc.block(i).start(), layers[i].parent),
    };
    // The drop target being hovered, unless dropping there changes nothing.
    let target = app.layer_drag.and_then(|d| match d.target {
        Some(DropTarget::Slot(slot, parent)) if d.doc == open.id => {
            let from = doc.index_of_layer(d.layer)?;
            let block = doc.block(from);
            let (start, end) = (*block.start(), *block.end());
            let noop = (slot >= start && slot <= end + 1 && parent == layers[from].parent)
                || (slot > start && slot <= end)
                || parent.is_some_and(|p| doc.is_within(p, d.layer));
            (!noop).then_some(DropTarget::Slot(slot, parent))
        }
        _ => None,
    });
    let line = |on: bool| {
        let color = if on { ACCENT_PRIMARY } else { iced::Color::TRANSPARENT };
        container(space()).width(Fill).height(2).style(style::fill(color))
    };
    for (k, &(index, layer)) in rows.iter().enumerate() {
        let name: Element<'_, Message> = match &app.renaming {
            Some((id, text)) if *id == layer.id => text_input("Layer name", text)
                .id(RENAME_INPUT)
                .size(TEXT_SIZE)
                .padding([2, 4])
                .on_input(Message::RenameInput)
                .on_submit(Message::CommitRename)
                .into(),
            // The name handles its own clicks (double-click renames), so it must also start
            // the select/drag itself, or only the thumbnail would.
            _ => mouse_area({
                // "fx" marks layers with effects, like Photoshop.
                let mut r = row![].spacing(6).align_y(Vertical::Center);
                // "T" marks editable text layers.
                if layer.text.as_ref().is_some_and(|t| t.is_current(&layer.pixels)) {
                    r = r.push(text("T").size(TEXT_SIZE).font(jd_ui::widget::bold()).color(ACCENT_PRIMARY));
                }
                if let Some(g) = layer.group {
                    r = r.push(
                        button(text(if g.open { "▾" } else { "▸" }).size(TEXT_SIZE))
                            .padding(Padding::from([0, 2]))
                            .style(style::ghost(false))
                            .on_press(Message::ToggleGroupOpen(layer.id)),
                    );
                }
                r = r.push(text(&layer.name).size(TEXT_SIZE).width(Fill));
                if !layer.style.is_empty() {
                    r = r.push(text("fx").size(TEXT_SIZE_SMALL).font(jd_ui::widget::bold()).color(TEXT_SECONDARY));
                }
                r
            })
                .on_press(Message::LayerPress(layer.id))
                .on_double_click(Message::StartRename(layer.id))
                .into(),
        };
        let thumb_image = |h: Option<&iced::widget::image::Handle>| -> Element<'_, Message> {
            match h {
                Some(handle) => image(handle.clone()).width(40).height(28).content_fit(iced::ContentFit::Contain).into(),
                None => space().into(),
            }
        };
        // Pressing a thumbnail picks what edits go to (and still selects / drags the layer).
        let thumb: Element<'_, Message> = if layer.is_group() {
            // Groups: a folder; double-click (or the arrow by the name) opens or closes it.
            mouse_area(container(icon(Icon::Folder, 18.0, iced::Color::from_rgb8(0x30, 0x30, 0x30))).center(Fill))
                .on_press(Message::ThumbPress(layer.id, false))
                .on_double_click(Message::ToggleGroupOpen(layer.id))
                .into()
        } else if layer.adjustment.is_some() {
            // Adjustment layers have no pixels: an icon; double-click edits the adjustment.
            mouse_area(container(icon(Icon::Adjust, 18.0, iced::Color::from_rgb8(0x30, 0x30, 0x30))).center(Fill))
                .on_press(Message::ThumbPress(layer.id, false))
                .on_double_click(Message::EditAdjustment(layer.id))
                .into()
        } else {
            mouse_area(thumb_image(open.thumbs.get(&layer.id).map(|t| &t.1))).on_press(Message::ThumbPress(layer.id, false)).into()
        };
        let mask = layer.mask.as_ref().map(|m| {
            let mut s = iced::widget::stack![thumb_image(open.mask_thumbs.get(&layer.id).map(|t| &t.1))];
            if !m.enabled {
                // Switched off: a red cross over it, like Photoshop.
                s = s.push(container(text("✕").size(22).color(iced::Color::from_rgb8(0xE5, 0x3E, 0x3E))).center(Fill));
            }
            mouse_area(container(s).width(40).height(28)).on_press(Message::ThumbPress(layer.id, true)).into()
        });
        let mask_targeted = Some(layer.id) == active_id && doc.editing_mask();
        let row = layer_row(
            doc.depth(layer.id) as f32 * 16.0,
            name,
            thumb,
            mask,
            mask_targeted,
            layer.visible,
            layer.locked,
            Some(layer.id) == active_id,
            // Pressing a row selects it and may start a drag (reorder, or onto a tab).
            Message::LayerPress(layer.id),
            Message::ToggleVisible(layer.id),
            Message::ToggleLockOf(layer.id),
            Message::OpenLayerMenu(layer.id),
        );
        // The line between this row and the one above is where both "above this row" and
        // "below the row above" drop.
        let top = target.is_some_and(|t| t == upper(index) || (k > 0 && t == lower(rows[k - 1].0)));
        let mut cell = column![line(top), row];
        if k + 1 == rows.len() {
            cell = cell.push(line(target.is_some_and(|t| t == lower(index))));
        }
        let (up, down) = (upper(index), lower(index));
        list = list.push(mouse_area(cell).on_move(move |p| Message::DragOver(if p.y < ROW_HALF { up } else { down })));
    }

    let mut actions = row![space::horizontal()].spacing(6).align_y(Vertical::Center);
    // Dimmed icons are for features that don't exist yet (their tooltips say so).
    for (ic, msg, tip) in [
        (Icon::Link, None, "Link layers (not available yet)"),
        (Icon::Fx, Some(Message::ShowDialog(crate::app::DialogKind::BlendingOptions)), "Layer style"),
        (Icon::Mask, Some(Message::AddMask), "Add layer mask (Alt: hide all)"),
        (Icon::Adjust, Some(Message::OpenAdjustMenu), "New adjustment layer"),
        (Icon::Folder, Some(Message::NewGroup), "New group (Ctrl+G groups the active layer)"),
        (Icon::Plus, Some(Message::AddLayer), "New layer"),
        (Icon::Trash, Some(Message::DeleteLayer), "Delete layer"),
    ] {
        actions = actions.push(hover_tip(icon_button(ic, 14.0, false, msg), tip, tooltip::Position::Top));
    }

    let content = column![
        container(column![controls, locks].spacing(8)).padding(Padding::from([0, 10]).bottom(8)),
        hline(BORDER_DEFAULT),
        scrollable(list).height(Fill),
        hline(BORDER_DEFAULT),
        container(actions).padding(Padding::from([4, 8])).style(style::fill(BG_PANEL_HEADER)),
    ];

    // Own layout instead of `panel` so layer rows run edge to edge.
    column![
        panel_header_grip(&["Layers", "Channels", "Paths"], 0, Message::LayersTab, Message::PanelGrab(Panel::Layers), Message::OpenPanelMenu(Panel::Layers)),
        container(content).padding(Padding::ZERO.top(8)).height(Fill),
    ]
    .height(Fill)
    .into()
}

/// The panel that slides out next to the collapsed-panel strip (History for now).
pub fn strip_panel(app: &Photo) -> Element<'_, Message> {
    if app.open_strip != Some(0) {
        return space().into();
    }
    let mut list = column![];
    if let Some(open) = app.doc() {
        let (done, undone) = open.history.labels();
        let first = if open.path.as_deref().is_some_and(jd_io::is_document) { "Open" } else { "Open Image" };
        let entries = std::iter::once((first, true))
            .chain(done.iter().map(|l| (*l, true)))
            .chain(undone.iter().map(|l| (*l, false)));
        for (i, (label_text, applied)) in entries.enumerate() {
            let current = i == done.len();
            let color = if applied { TEXT_PRIMARY } else { TEXT_MUTED };
            list = list.push(
                button(text(label_text).size(TEXT_SIZE).color(color))
                    .width(Fill)
                    .padding(Padding::from([5, 10]))
                    .style(style::row(current))
                    .on_press(Message::JumpHistory(i)),
            );
        }
    } else {
        list = list.push(container(label("No document open.")).padding(10));
    }
    row![
        column![
            panel_header(&["History"], 0, |_| Message::Noop),
            scrollable(list).height(Fill),
        ]
        .width(220)
        .height(Fill),
        jd_ui::widget::vline(Fill, BORDER_DEFAULT),
    ]
    .into()
}
