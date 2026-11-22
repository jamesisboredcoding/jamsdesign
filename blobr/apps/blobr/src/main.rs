mod app;
mod clipboard;
mod tool;
mod view;

/// Display name, used in the title bar.
pub const APP_NAME: &str = "Blobr";

fn main() -> iced::Result {
    iced::application(app::Blobr::boot, app::Blobr::update, view::view)
        .title(|app: &app::Blobr| format!("{} - {APP_NAME}", app.file_name()))
        .theme(|_: &app::Blobr| jd_ui::theme())
        .subscription(app::Blobr::subscription)
        .settings(iced::Settings {
            default_text_size: jd_ui::tokens::TEXT_SIZE.into(),
            antialiasing: true,
            ..Default::default()
        })
        .window(iced::window::Settings {
            size: iced::Size::new(1600.0, 960.0),
            min_size: Some(iced::Size::new(1000.0, 600.0)),
            // We draw our own title bar and window controls.
            decorations: false,
            icon: window_icon(),
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: APP_ID.to_owned(),
                ..Default::default()
            },
            ..Default::default()
        })
        .centered()
        .exit_on_close_request(false)
        .run()
}

/// Wayland app id; also the name of the .desktop file and the installed icon.
pub const APP_ID: &str = "dev.jamsdesign.Blobr";

/// The Blobr logo (the suite's shared artwork in the repo root `img/`).
pub const LOGO_SVG: &[u8] = include_bytes!("../../../../img/blobr.svg");

/// The logo rendered to pixels for the window icon (used by X11; Wayland uses the .desktop file).
fn window_icon() -> Option<iced::window::Icon> {
    const SIZE: u32 = 128;
    let tree = resvg::usvg::Tree::from_data(LOGO_SVG, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(SIZE, SIZE)?;
    let scale = SIZE as f32 / tree.size().width().max(tree.size().height());
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia is premultiplied; window icons are straight alpha.
    let rgba = pixmap.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    });
    iced::window::icon::from_rgba(rgba.collect(), SIZE, SIZE).ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn logo_renders_to_a_window_icon() {
        assert!(super::window_icon().is_some());
    }
}
