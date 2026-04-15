mod app;
mod camera;
mod config;
mod preview;

use cosmic::app::Settings;

fn main() -> cosmic::iced::Result {
    env_logger::init();

    let settings = Settings::default();
    cosmic::app::run::<app::App>(settings, ())
}
