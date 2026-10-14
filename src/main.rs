mod app;
mod appearance;
mod drag_area;
mod explorer;
mod file_dialog;
mod file_manager;
mod viewer;
mod volume_session;

use app::Enote;
use volume_session::Startup;

fn main() -> iced::Result {
    let startup = Startup::discover();
    if !startup.warnings.is_empty() {
        eprintln!("Startup: {}", startup.warnings);
    }
    let settings = startup.settings();
    let window_size = startup
        .window
        .as_ref()
        .map(|w| (w.width, w.height))
        .unwrap_or((settings.window.width as f32, settings.window.height as f32));
    let position = startup
        .window
        .as_ref()
        .and_then(|w| w.position)
        .map(|[x, y]| iced::window::Position::Specific(iced::Point::new(x, y)))
        .unwrap_or_default();
    iced::application(
        move || {
            let mut app = Enote::from_startup(startup.clone());
            let task = app.startup_tasks();
            (app, task)
        },
        Enote::update,
        Enote::view,
    )
    .title("eNote")
    .window_size(window_size)
    .position(position)
    .font(lucide_icons::LUCIDE_FONT_BYTES)
    .theme(|_: &Enote| appearance::theme())
    .exit_on_close_request(false)
    .subscription(Enote::subscription)
    .run()
}
