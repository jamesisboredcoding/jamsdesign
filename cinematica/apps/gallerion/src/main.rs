mod adjust;
mod app;
mod tablet;
mod transform;
mod tool;
mod view;

/// Display name, used in the title bar and menu bar.
pub const APP_NAME: &str = "Gallerion";

fn main() -> iced::Result {
    iced::application(app::Photo::new, app::Photo::update, view::view)
        .title(|app: &app::Photo| match app.document() {
            Some(doc) => format!("{} - {APP_NAME}", doc.name),
            None => APP_NAME.to_owned(),
        })
        .theme(|_: &app::Photo| jd_ui::theme())
        .subscription(app::Photo::subscription)
        .settings(iced::Settings {
            default_text_size: jd_ui::tokens::TEXT_SIZE.into(),
            antialiasing: true,
            ..Default::default()
        })
        .window(iced::window::Settings {
            size: iced::Size::new(1600.0, 960.0),
            min_size: Some(iced::Size::new(900.0, 560.0)),
            // We draw our own title bar and window controls.
            decorations: false,
            ..Default::default()
        })
        .centered()
        // Closing asks about unsaved documents first (see Message::RequestQuit).
        .exit_on_close_request(false)
        .run()
}
