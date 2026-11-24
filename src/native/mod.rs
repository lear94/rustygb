//! Desktop front-end: window, keyboard, frame pacing and audio around the core.

mod app;
mod audio;
mod display;
mod frame_clock;
mod keys;
mod session;

use app::App;
use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;
use winit::event_loop::EventLoop;

/// Runs until the window closes.
pub(crate) fn run() -> Result<ExitCode, Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    let mut app = App::new(env::args_os().nth(1).map(PathBuf::from));
    event_loop.run_app(&mut app)?;
    app.into_result()
}
