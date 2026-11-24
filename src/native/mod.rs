//! Desktop front-end: window, keyboard, frame pacing and audio around the core.

mod app;
mod audio;
mod discard;
mod display;
mod frame_clock;
mod keys;
mod report;
mod session;

use app::{App, Terminate};
use report::report;
use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;
use winit::event_loop::EventLoop;

/// Runs until the window closes.
pub(crate) fn run() -> Result<ExitCode, Box<dyn Error>> {
    let event_loop = EventLoop::<Terminate>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    // A failure means the loop already ended, and it saves on its way out.
    let handler = move || drop(proxy.send_event(Terminate));
    if let Err(e) = ctrlc::set_handler(handler) {
        report(format_args!("Signals will not save the game: {e}"));
    }
    let mut app = App::new(env::args_os().nth(1).map(PathBuf::from));
    let ran = event_loop.run_app(&mut app);
    app.into_result(ran.err().map(Into::into))
}
