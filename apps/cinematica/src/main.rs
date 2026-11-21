mod app;
mod tool;
mod view;

/// Display name, used in the title bar.
pub const APP_NAME: &str = "Cinematica";

fn main() -> iced::Result {
    iced::application(app::Editor::new, app::Editor::update, view::view)
        .title(|app: &app::Editor| format!("{} - {APP_NAME}", app.project_name))
        .theme(|_: &app::Editor| jd_ui::theme())
        .subscription(app::Editor::subscription)
        .settings(iced::Settings {
            default_text_size: jd_ui::tokens::TEXT_SIZE.into(),
            antialiasing: true,
            ..Default::default()
        })
        .window(iced::window::Settings {
            size: iced::Size::new(1600.0, 960.0),
            min_size: Some(iced::Size::new(1100.0, 640.0)),
            // We draw our own title bar and window controls.
            decorations: false,
            ..Default::default()
        })
        .centered()
        .exit_on_close_request(false)
        .run()
}
