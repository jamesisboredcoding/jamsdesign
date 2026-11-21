//! Window chrome: menu bar, tool options bar, toolbar, document tabs, status bar, panel strip.

use iced::alignment::Vertical;
use iced::widget::{button, column, container, mouse_area, pick_list, row, slider, space, stack, text, tooltip};
use iced::{Element, Fill, Padding, Point};
use jd_raster::selection::Combine;
use jd_raster::{BlendMode, BlendSpace};
use jd_ui::widget::{bold, field_box, hline, hover_tip, icon_button, label, labeled, tool_button, value, vline};
use jd_ui::{Icon, icon, style, tokens::*};

use super::to_color;
use crate::app::{Message, Photo, PopupKind, Zoom};
use crate::tool::Tool;
use crate::APP_NAME;

pub const MENUS: &[&str] = &[
    "File", "Edit", "Image", "Layer", "Select", "Filter", "View", "Window", "Help",
];

/// Our own title bar (the window has no system decorations): app badge, menus, title,
/// actions and window controls. Empty space drags the window; double-click maximises.
pub fn title_bar(app: &Photo) -> Element<'_, Message> {
    let badge = container(text("Jd").size(10).font(bold()))
        .center(20)
        .style(|_| container::Style {
            background: Some(
                iced::gradient::Linear::new(std::f32::consts::FRAC_PI_4)
                    .add_stop(0.0, iced::Color::from_rgb8(0x4C, 0x8D, 0xFF))
                    .add_stop(1.0, iced::Color::from_rgb8(0x8C, 0x4D, 0xF2))
                    .into(),
            ),
            text_color: Some(TEXT_PRIMARY),
            border: iced::border::rounded(4),
            ..Default::default()
        });

    let mut menus = row![badge, space().width(8)].align_y(Vertical::Center).spacing(2);
    for m in MENUS {
        // Real dropdown menus come later; File opens the file dialog for now.
        let i = MENUS.iter().position(|x| x == m).expect("menu title");
        let open = matches!(app.popup, Some(p) if p.kind == PopupKind::MenuBar(i));
        menus = menus.push(
            mouse_area(
                button(text(*m).size(TEXT_SIZE))
                    .padding(Padding::from([4, 8]))
                    .style(style::ghost(open))
                    .on_press(Message::MenuTitle(i)),
            )
            .on_move(move |p| Message::MenuPointer(i, p)),
        );
    }
    // Workspace switcher: opens under the pointer, like the menus.
    let workspace = button(
        row![value(app.workspace.name()), icon(Icon::Chevron, 12.0, TEXT_SECONDARY)]
            .spacing(6)
            .align_y(Vertical::Center),
    )
    .padding(Padding::from([3, 8]))
    .style(style::ghost(matches!(app.popup, Some(p) if p.kind == PopupKind::Workspace)))
    .on_press(Message::OpenPopup(PopupKind::Workspace, Point::new(app.title_pointer.x - 60.0, 30.0)));
    let search = hover_tip(
        button(icon(Icon::Zoom, 14.0, ICON)).padding(4).style(style::ghost(false)).on_press(Message::OpenPalette),
        "Search commands (Ctrl+F)",
        tooltip::Position::Bottom,
    );
    let export = button(
        row![icon(Icon::Share, 12.0, TEXT_PRIMARY), text("Export").size(TEXT_SIZE_SMALL).font(bold())]
            .spacing(6)
            .align_y(Vertical::Center),
    )
    .padding(Padding::from([4, 10]))
    .style(style::primary)
    .on_press_maybe(app.document().map(|_| Message::Export));

    let title = match app.document() {
        Some(doc) => format!("{} - {APP_NAME}", doc.name),
        None => format!("{APP_NAME} - jamsdesign"),
    };

    let control = |ic: Icon, danger: bool, msg: Message| {
        button(container(icon(ic, 14.0, ICON)).center(iced::Fill))
            .width(44)
            .height(30)
            .padding(0)
            .style(style::window_control(danger))
            .on_press(msg)
    };
    // Fullscreen is F11 (no button: maximise covers the common case).
    let controls = row![
        control(Icon::Minimize, false, Message::Minimize),
        control(
            if app.maximized { Icon::Restore } else { Icon::Maximize },
            false,
            Message::ToggleMaximize
        ),
        control(Icon::Close, true, Message::RequestQuit),
    ];

    let bar = container(
        row![
            container(menus).padding(Padding::ZERO.left(10)),
            space::horizontal(),
            text(title).size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
            space::horizontal(),
            row![search, workspace, export]
                .spacing(12)
                .align_y(Vertical::Center),
            space().width(12),
            controls,
        ]
        .align_y(Vertical::Center),
    )
    .height(30)
    .align_y(Vertical::Center)
    .style(style::fill(BG_PANEL_HEADER));

    // Buttons inside capture their own clicks, so presses that reach here are on empty space.
    mouse_area(bar)
        .on_press(Message::TitlePressed)
        .on_move(Message::TitleMoved)
        .on_release(Message::TitleReleased)
        .on_double_click(Message::ToggleMaximize)
        .into()
}

fn percent_slider<'a>(
    name: &'a str,
    v: f32,
    on_change: impl Fn(f32) -> Message + 'a,
) -> Element<'a, Message> {
    labeled(
        name,
        row![
            field_box(value(format!("{:.0}%", v * 100.0))).width(46),
            slider(0.0..=1.0, v, on_change).step(0.01_f32).width(70).style(style::slider),
        ]
        .spacing(6)
        .align_y(Vertical::Center),
    )
}

/// A small on/off text button with a hover tip.
fn toggle<'a>(label_text: &'a str, on: bool, msg: Message, tip: &'a str) -> Element<'a, Message> {
    hover_tip(
        button(text(label_text).size(TEXT_SIZE_SMALL).color(if on { TEXT_PRIMARY } else { TEXT_MUTED }))
            .padding(Padding::from([3, 8]))
            .style(style::ghost(on))
            .on_press(msg),
        tip,
        tooltip::Position::Bottom,
    )
}

pub fn options_bar(app: &Photo) -> Element<'_, Message> {
    let tool = app.tool;
    let mut bar = row![
        icon(Icon::Home, 16.0, ICON),
        vline(20.0, BORDER_SUBTLE),
        row![icon(tool.icon(), 16.0, ICON), icon(Icon::Chevron, 10.0, TEXT_SECONDARY)]
            .spacing(2)
            .align_y(Vertical::Center),
    ]
    .spacing(14)
    .align_y(Vertical::Center);

    if let Some(t) = app.transform.as_ref().filter(|t| t.kind == crate::transform::Kind::Free) {
        let p = t.params;
        let commit = button(text("Commit").size(TEXT_SIZE_SMALL).font(bold()))
            .padding(Padding::from([3, 10]))
            .style(style::primary)
            .on_press(Message::CommitTransform);
        let cancel = button(text("Cancel").size(TEXT_SIZE_SMALL))
            .padding(Padding::from([3, 10]))
            .style(style::ghost(false))
            .on_press(Message::CancelTransform);
        bar = bar
            .push(labeled("W:", field_box(value(format!("{:.1}%", p.sx * 100.0)))))
            .push(labeled("H:", field_box(value(format!("{:.1}%", p.sy * 100.0)))))
            .push(labeled("Angle:", field_box(value(format!("{:.1}°", p.angle.to_degrees())))))
            .push(label("Shift: free aspect / 15° steps · Alt: from centre · Enter commits, Esc cancels"))
            .push(space::horizontal())
            .push(cancel)
            .push(commit);
        return options_container(bar);
    }

    if tool.is_selection() {
        let mode = |name: &'static str, m: Combine, tip: &'static str| {
            toggle(name, app.select.mode == m, Message::SetSelectMode(m), tip)
        };
        bar = bar
            .push(mode("New", Combine::Replace, "New selection"))
            .push(mode("Add", Combine::Add, "Add to selection (or hold Shift)"))
            .push(mode("Subtract", Combine::Subtract, "Subtract from selection (or hold Alt)"))
            .push(mode("Intersect", Combine::Intersect, "Intersect with selection (or hold Shift+Alt)"))
            .push(vline(20.0, BORDER_SUBTLE));
        match tool {
            Tool::Marquee | Tool::EllipticalMarquee => {
                bar = bar
                    .push(toggle("Rectangle", tool == Tool::Marquee, Message::SelectTool(Tool::Marquee), "Rectangular Marquee (Shift+M switches)"))
                    .push(toggle("Ellipse", tool == Tool::EllipticalMarquee, Message::SelectTool(Tool::EllipticalMarquee), "Elliptical Marquee (Shift+M switches)"))
                    .push(label("Shift: square · Alt: from centre"));
            }
            Tool::PolygonalLasso => {
                bar = bar.push(label(
                    "Click to add points · Double-click, Enter or click the start to close · Backspace removes a point · Esc cancels",
                ));
            }
            Tool::Wand => {
                let tol = app.select.tolerance;
                bar = bar
                    .push(labeled(
                        "Tolerance:",
                        row![
                            field_box(value(tol.to_string())).width(40),
                            slider(0.0..=255.0, tol as f32, Message::WandTolerance).width(90).style(style::slider),
                        ]
                        .spacing(6)
                        .align_y(Vertical::Center),
                    ))
                    .push(toggle("Contiguous", app.select.contiguous, Message::WandContiguous(!app.select.contiguous), "Only select connected pixels"))
                    .push(toggle(
                        "Sample All Layers",
                        app.select.all_layers,
                        Message::WandAllLayers(!app.select.all_layers),
                        "Pick by the combined image instead of the active layer",
                    ));
            }
            _ => {}
        }
        bar = bar.push(label("Ctrl+D deselect · Ctrl+Shift+I inverse"));
        return options_container(bar);
    }

    if tool == Tool::Move {
        bar = bar.push(label(
            "Drag to move the layer or selection · Arrows nudge 1 px (Shift: 10) · Ctrl+T: Free Transform",
        ));
        return options_container(bar);
    }

    if tool.uses_brush() {
        let b = &app.brush;
        bar = bar
            .push(labeled(
                "Size:",
                row![
                    field_box(value(format!("{:.0} px", b.size))).width(58),
                    slider(1.0..=500.0, b.size, Message::BrushSize).width(80).style(style::slider),
                ]
                .spacing(6)
                .align_y(Vertical::Center),
            ))
            .push(percent_slider("Hardness:", b.hardness, Message::BrushHardness))
            .push(vline(20.0, BORDER_SUBTLE))
            .push(labeled(
                "Mode:",
                pick_list(BlendMode::ALL, Some(b.mode), Message::BrushMode)
                    .text_size(TEXT_SIZE_SMALL)
                    .padding(Padding::from([3, 6]))
                    .width(110)
                    .style(style::pick_list),
            ))
            .push(percent_slider("Opacity:", b.opacity, Message::BrushOpacity))
            .push(percent_slider("Flow:", b.flow, Message::BrushFlow))
            .push(percent_slider("Smoothing:", b.smoothing, Message::BrushSmoothing))
            .push(vline(20.0, BORDER_SUBTLE));
        if matches!(tool, Tool::Dodge | Tool::Burn) {
            bar = bar.push(percent_slider("Exposure:", b.exposure, Message::BrushExposure)).push(vline(20.0, BORDER_SUBTLE));
        }
        if tool.clones() {
            bar = bar.push(label(if app.clone_source.is_some() { "Alt+click: new source" } else { "Alt+click to set the source" }));
        }
        bar = bar
            .push(toggle(
                "Size",
                b.pressure_size,
                Message::PressureSize(!b.pressure_size),
                "Pen pressure controls size",
            ))
            .push(toggle(
                "Opacity",
                b.pressure_opacity,
                Message::PressureOpacity(!b.pressure_opacity),
                "Pen pressure controls opacity",
            ));
    } else if tool == Tool::Gradient {
        use jd_raster::gradient::Shape;
        bar = bar
            .push(toggle("Linear", app.gradient_shape == Shape::Linear, Message::GradientShape(Shape::Linear), "Linear gradient"))
            .push(toggle("Radial", app.gradient_shape == Shape::Radial, Message::GradientShape(Shape::Radial), "Radial gradient"))
            .push(vline(20.0, BORDER_SUBTLE))
            .push(percent_slider("Opacity:", app.brush.opacity, Message::BrushOpacity))
            .push(label("Drag: foreground to background colour · Shift snaps to 45°"));
    } else if tool.is_shape() {
        bar = bar.push(label("Drag to draw a filled shape on a new layer, in the foreground colour · Shift: square · Alt: from centre"));
    } else if tool == Tool::Crop {
        bar = bar.push(label(if app.crop_box.is_some() {
            "Enter to crop · Esc to cancel · Drag again for a new box"
        } else {
            "Drag a crop box (Shift: square)"
        }));
    } else {
        bar = bar.push(label(format!("{}: options for this tool come with its implementation", tool.name())));
    }
    if app.tablet_allowed {
        bar = bar.push(space::horizontal()).push(toggle(
            if app.pen_pressure { "Pen pressure: On" } else { "Pen pressure: Off" },
            app.pen_pressure,
            Message::ToggleTablet,
            "Use the drawing tablet's pressure and eraser end. Off: the pen paints like a mouse.",
        ));
    }

    options_container(bar)
}

fn options_container<'a>(bar: iced::widget::Row<'a, Message>) -> Element<'a, Message> {
    container(bar)
        .height(38)
        .width(Fill)
        .padding(Padding::from([0, 10]))
        .align_y(Vertical::Center)
        .style(style::fill(BG_PANEL))
        .into()
}

pub fn toolbar(app: &Photo) -> Element<'_, Message> {
    let mut col = column![
        container(iced::widget::space())
            .width(20)
            .height(3)
            .style(style::fill(BORDER_SUBTLE)),
    ]
    .spacing(2)
    .width(Fill)
    .align_x(iced::Alignment::Center)
    .padding(Padding::from([6, 0]));

    let mut slot = 0;
    for (i, group) in crate::tool::GROUPS.iter().enumerate() {
        if i > 0 {
            col = col.push(container(hline(BORDER_SUBTLE)).width(24).padding(Padding::from([3, 0])));
        }
        for tools in *group {
            let t = app.slot_tools[slot];
            let has_group = tools.len() > 1;
            let b = tool_button(t.icon(), tools.contains(&app.tool), has_group, t.tooltip(), Message::SelectTool(t));
            // Right-click opens the slot's alternates (only slots that have some).
            col = col.push(if has_group {
                let at = Point::new(46.0, TOOLBAR_TOP + app.toolbar_pointer.y - 14.0);
                mouse_area(b).on_right_press(Message::OpenPopup(PopupKind::ToolSlot(slot), at)).into()
            } else {
                b
            });
            slot += 1;
        }
    }

    // Foreground / background swatches: click fg/bg area to swap (X), reset with D.
    let swatch = |c: iced::Color| {
        container(space())
            .width(18)
            .height(18)
            .style(move |_| container::Style {
                background: Some(c.into()),
                border: iced::Border {
                    color: iced::Color::from_rgb8(0xD0, 0xD0, 0xD0),
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
    };
    let swatches = button(stack![
        container(swatch(to_color(app.bg))).padding(Padding::ZERO.left(14).top(14)),
        container(swatch(to_color(app.fg))).padding(Padding::ZERO.left(2).top(2)),
    ])
    .padding(0)
    .width(36)
    .height(36)
    .style(style::ghost(false))
    .on_press(Message::SwapColors);

    col = col
        .push(space().height(8))
        .push(swatches)
        .push(icon_button(Icon::Mask, 14.0, false, Some(Message::Noop)));

    mouse_area(container(col).width(44).height(Fill).style(style::fill(BG_PANEL)))
        .on_move(Message::ToolbarPointer)
        .into()
}

/// Window y of the toolbar's top edge: title bar (30) + line + options bar (38) + line.
pub const TOOLBAR_TOP: f32 = 30.0 + 1.0 + 38.0 + 1.0;

pub fn document_tabs(app: &Photo) -> Element<'_, Message> {
    let mut tabs = row![];
    for (i, open) in app.docs.iter().enumerate() {
        let is_active = i == app.active;
        // "*" marks unsaved changes, like Photoshop.
        let dirty = if open.is_dirty() { "*" } else { "" };
        let label = if is_active {
            let layer = open.doc.active().map_or("-", |l| l.name.as_str());
            format!("{}{dirty} @ {:.1}% ({layer}, RGB/8)", open.doc.name, app.zoom_factor() * 100.0)
        } else {
            format!("{}{dirty}", open.doc.name)
        };
        let close = button(icon(Icon::Close, 12.0, ICON))
            .padding(2)
            .style(style::ghost(false))
            .on_press(Message::RequestCloseDoc(i));
        tabs = tabs
            .push(mouse_area(
                button(
                    row![text(label).size(TEXT_SIZE_SMALL), close]
                        .spacing(10)
                        .align_y(Vertical::Center),
                )
                .padding(Padding::from([5, 12]).right(6))
                .style(style::tab(is_active))
                .on_press(Message::SelectDoc(i)),
            )
            // Dragging a layer over a tab switches to that document (drop copies it there).
            .on_enter(Message::DragOver(crate::app::DropTarget::Tab(i))))
            .push(vline(28.0, BORDER_DEFAULT));
    }
    if app.loading > 0 {
        tabs = tabs.push(container(label("Opening…")).padding(Padding::from([7, 12])));
    }

    container(tabs)
        .width(Fill)
        .height(28)
        .style(style::fill(BG_PANEL_HEADER))
        .into()
}

pub fn status_bar(app: &Photo) -> Element<'_, Message> {
    let mut bar = row![].spacing(12).align_y(Vertical::Center);
    if let (Some(open), Some(doc)) = (app.doc(), app.document()) {
        let fit = button(text("Fit").size(TEXT_SIZE_SMALL))
            .padding(Padding::from([2, 6]))
            .style(style::ghost(open.zoom == Zoom::Fit))
            .on_press(Message::ZoomTo(Zoom::Fit));
        let actual = button(text("100%").size(TEXT_SIZE_SMALL))
            .padding(Padding::from([2, 6]))
            .style(style::ghost(open.zoom == Zoom::Fixed(1.0)))
            .on_press(Message::ZoomTo(Zoom::Fixed(1.0)));
        bar = bar
            .push(field_box(value(format!("{:.2}%", app.zoom_factor() * 100.0))))
            .push(fit)
            .push(actual)
            .push(label(format!("{} px × {} px", doc.width, doc.height)))
            .push(label("sRGB · 8 bit"))
            .push(blend_space_toggle(doc.blend_space));
    }
    bar = bar.push(space::horizontal());
    if let Some(drag) = app.layer_drag
        && drag.target.is_some_and(|t| t != crate::app::DropTarget::Layer(drag.layer))
    {
        bar = bar.push(label("Drop on a layer to reorder · on another document's tab or canvas to copy it there"));
    } else if let Some(notice) = &app.notice {
        let color = if notice.error { DANGER } else { TEXT_SECONDARY };
        bar = bar.push(text(&notice.text).size(TEXT_SIZE_SMALL).color(color));
    }
    if let Some(p) = app.cursor
        && app.document().is_some()
    {
        bar = bar.push(label(format!("X: {:.0}  Y: {:.0}", p.x, p.y)));
    }
    if let Some(doc) = app.document() {
        let n = doc.layers().len();
        bar = bar.push(label(format!("{n} layer{}", if n == 1 { "" } else { "s" })));
    }

    container(bar)
        .height(24)
        .width(Fill)
        .padding(Padding::from([0, 10]))
        .align_y(Vertical::Center)
        .style(style::fill(BG_PANEL_HEADER))
        .into()
}

/// Click to switch how layers blend (undoable, saved with the document).
fn blend_space_toggle<'a>(space: BlendSpace) -> Element<'a, Message> {
    let (label_text, next) = match space {
        BlendSpace::Perceptual => ("Blending: Perceptual", BlendSpace::Linear),
        BlendSpace::Linear => ("Blending: Linear", BlendSpace::Perceptual),
    };
    tooltip(
        button(text(label_text).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY))
            .padding(Padding::from([2, 6]))
            .style(style::ghost(false))
            .on_press(Message::SetBlendSpace(next)),
        container(
            text("Perceptual blends like Photoshop. Linear blends physically correct light.")
                .size(TEXT_SIZE_SMALL),
        )
        .padding(6)
        .style(style::input),
        tooltip::Position::Top,
    )
    .into()
}

pub fn collapsed_panels(app: &Photo) -> Element<'_, Message> {
    let items = [Icon::History, Icon::Brushes, Icon::Type, Icon::Info, Icon::Fx];
    let mut col = column![].spacing(6).width(Fill).align_x(iced::Alignment::Center).padding(Padding::from([8, 0]));
    for (i, ic) in items.into_iter().enumerate() {
        col = col.push(
            icon_button(ic, 16.0, app.open_strip == Some(i), Some(Message::ToggleStrip(i)))
                .padding(6),
        );
    }
    container(col).width(36).height(Fill).style(style::fill(BG_PANEL)).into()
}
