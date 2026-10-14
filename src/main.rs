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
    let window_size = (settings.window.width as f32, settings.window.height as f32);
    iced::application(
        move || Enote::from_startup(startup.clone()),
        Enote::update,
        Enote::view,
    )
    .title("eNote")
    .window_size(window_size)
    .font(lucide_icons::LUCIDE_FONT_BYTES)
    .theme(|_: &Enote| appearance::theme())
    .exit_on_close_request(false)
    .subscription(Enote::subscription)
    .run()
}
