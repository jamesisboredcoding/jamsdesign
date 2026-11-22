//! Right-hand panel dock: Color, Properties, Layers.

use iced::alignment::Vertical;
use iced::widget::{button, column, container, image, mouse_area, pick_list, row, scrollable, slider, space, text, text_input, tooltip};
use iced::{Color, Element, Fill, Length, Padding, Shrink};
use jd_core::Hsv;
use jd_raster::BlendMode;
use jd_ui::widget::{bold, field_box, hline, icon_button, label, layer_row, panel, panel_header, value};
use jd_ui::{Icon, icon, style, tokens::*};

use super::{color, to_color};
use crate::app::{DropTarget, Message, Photo, RENAME_INPUT};

pub fn view(app: &Photo) -> Element<'_, Message> {
    // Window menu toggles which panels show; Layers takes the remaining height.
    let mut col = column![].height(Fill);
    if app.panels.color {
        col = col.push(color_panel(app)).push(hline(BORDER_DEFAULT));
    }
    if app.panels.properties {
        col = col.push(properties_panel(app)).push(hline(BORDER_DEFAULT));
    }
    if app.panels.layers {
        col = col.push(layers_panel(app));
    }
    if !(app.panels.color || app.panels.properties || app.panels.layers) {
        // Minimal workspace: no dock at all.
        return space().into();
    }
    // Tracks the pointer so the layer right-click menu opens where you clicked.
    mouse_area(container(col).width(app.dock_width).height(Fill).style(style::fill(BG_PANEL)))
        .on_move(Message::DockPointer)
        .into()
}

fn color_panel(app: &Photo) -> Element<'_, Message> {
    let content: Element<'_, Message> = match app.color_tab {
        0 => {
            let [r, g, b] = app.fg.to_rgba().to_srgb8();
            let hex = app.fg.to_rgba().to_hex();
            let channel = |name, v: String| {
                field_box(row![text(name).size(10).color(TEXT_MUTED), value(v)].spacing(4))
                    .width(Fill)
            };
            column![
                row![color::sv_square(app.fg), color::hue_strip(app.fg)].spacing(8),
                row![
                    container(channel("#", hex)).width(Length::FillPortion(2)),
                    container(channel("R", r.to_string())).width(Length::FillPortion(1)),
                    container(channel("G", g.to_string())).width(Length::FillPortion(1)),
                    container(channel("B", b.to_string())).width(Length::FillPortion(1)),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into()
        }
        _ => swatches(),
    };
    panel(&["Color", "Swatches"], app.color_tab, Message::ColorTab, content, Shrink)
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
        (0, Some(layer)) => column![
            row![icon(Icon::Layers, 14.0, ICON), text(&layer.name).size(TEXT_SIZE_SMALL).font(bold())]
                .spacing(6)
                .align_y(Vertical::Center),
            row![
                label("Opacity").width(Fill),
                value(format!("{:.0}%", layer.opacity * 100.0))
            ],
            slider(0.0..=1.0, layer.opacity, Message::LayerOpacity)
                .step(0.01_f32)
                .on_release(Message::EndGesture)
                .style(style::slider),
            row![
                label("Blend").width(Fill),
                blend_picker(layer.blend),
            ]
            .align_y(Vertical::Center),
        ]
        .spacing(8)
        .into(),
        (0, None) => label("No layer selected").into(),
        _ => adjustments(app),
    };
    panel(&["Properties", "Adjustments"], app.props_tab, Message::PropsTab, content, Shrink)
}

/// The Adjustments tab: every adjustment and filter one click away (they apply to the active
/// layer, like the Image and Filter menus; adjustment layers aren't there yet).
fn adjustments(app: &Photo) -> Element<'_, Message> {
    use crate::adjust::OpKind::*;
    let editable = app.document().and_then(|d| d.active()).is_some_and(|l| !l.locked);
    let items = [
        ("Brightness/Contrast", BrightnessContrast),
        ("Levels", Levels),
        ("Curves", Curves),
        ("Hue/Saturation", HueSaturation),
        ("Threshold", Threshold),
        ("Posterize", Posterize),
        ("Invert", Invert),
        ("Desaturate", Desaturate),
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
        grid,
        label(if editable { "Applies to the active layer (Ctrl+Z undoes)." } else { "Select an unlocked layer to adjust." }),
    ]
    .spacing(8)
    .into()
}

/// The Channels tab: the composite image's channels. Clicking one loads it as a selection
/// (bright = selected), like Ctrl+clicking a channel in Photoshop.
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

fn blend_picker<'a>(mode: BlendMode) -> Element<'a, Message> {
    pick_list(BlendMode::ALL, Some(mode), Message::LayerBlend)
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
        return panel(&["Layers", "Channels"], app.layers_tab, Message::LayersTab, channels_panel(app), Fill);
    }

    let Some(doc) = app.document() else {
        return panel(
            &["Layers", "Channels"],
            app.layers_tab,
            Message::LayersTab,
            label("No document open."),
            Fill,
        );
    };
    let active = doc.active();
    let controls = row![
        blend_picker(active.map_or(BlendMode::Normal, |l| l.blend)),
        space::horizontal(),
        label("Opacity:"),
        field_box(value(format!("{:.0}%", active.map_or(100.0, |l| l.opacity * 100.0)))).width(52),
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
    let locks = row![
        label("Lock:"),
        icon_button(Icon::Brush, 13.0, false, None).padding(3),
        icon_button(Icon::Move, 13.0, false, None).padding(3),
        lock_all,
    ]
    .spacing(4)
    .align_y(Vertical::Center);

    let open = app.doc().expect("document() implies doc()");
    let active_id = doc.active_id();
    let mut list = column![];
    // Top of the stack first, like every layers panel.
    for layer in doc.layers().iter().rev() {
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
        let thumb: Element<'_, Message> = mouse_area(thumb_image(open.thumbs.get(&layer.id).map(|t| &t.1))).on_press(Message::ThumbPress(layer.id, false)).into();
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
        // Drop slots: the upper half of a row means "above it", the lower half "below it".
        // The accent line is drawn exactly where the layer will land.
        let index = doc.layers().iter().position(|l| l.id == layer.id).unwrap_or(0);
        let shown = app.layer_drag.and_then(|d| match d.target {
            Some(DropTarget::Slot(slot)) if d.doc == open.id => {
                let from = doc.layers().iter().position(|l| l.id == d.layer)?;
                // Dropping right next to itself changes nothing, so show no line.
                (slot != from && slot != from + 1).then_some(slot)
            }
            _ => None,
        });
        let line = |slot: usize| {
            let color = if shown == Some(slot) { ACCENT_PRIMARY } else { iced::Color::TRANSPARENT };
            container(space()).width(Fill).height(2).style(style::fill(color))
        };
        let mut cell = column![line(index + 1), row];
        if index == 0 {
            cell = cell.push(line(0));
        }
        list = list.push(mouse_area(cell).on_move(move |p| {
            Message::DragOver(DropTarget::Slot(if p.y < ROW_HALF { index + 1 } else { index }))
        }));
    }

    let mut actions = row![space::horizontal()].spacing(6).align_y(Vertical::Center);
    // Disabled icons are for features that don't exist yet.
    for (ic, msg) in [
        (Icon::Link, None),
        (Icon::Fx, Some(Message::ShowDialog(crate::app::DialogKind::BlendingOptions))),
        (Icon::Mask, Some(Message::AddMask)),
        (Icon::Adjust, None),
        (Icon::Folder, None),
        (Icon::Plus, Some(Message::AddLayer)),
        (Icon::Trash, Some(Message::DeleteLayer)),
    ] {
        actions = actions.push(icon_button(ic, 14.0, false, msg));
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
        panel_header(&["Layers", "Channels"], 0, Message::LayersTab),
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
