//! Application state and its reaction to window events.

use super::audio::AudioOutput;
use super::discard::{Action, Discard};
use super::display::Display;
use super::frame_clock::FrameClock;
use super::keys::Keys;
use super::report::report;
use super::session::Session;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::WindowId;

const WINDOW_TITLE: &str = "RustyGB";
/// Core output rate while no audio device consumes it.
const SILENT_SAMPLE_RATE_HZ: u32 = 48_000;
/// Opening a device can stall, so failures are not retried every frame.
const AUDIO_RETRY_PERIOD: Duration = Duration::from_secs(5);
/// Exit code of a session closed with unsaved progress.
const EXIT_UNSAVED: u8 = 2;

pub(super) struct App {
    /// ROM of the command line, until the window opens.
    initial_rom: Option<PathBuf>,
    display: Option<Display>,
    title: String,
    keys: Keys,
    audio: Option<AudioOutput>,
    /// When to look for a replacement of a lost device.
    audio_retry: Option<Instant>,
    session: Option<Session>,
    clock: FrameClock,
    discard: Discard,
    /// The last exit attempt could not save.
    unsaved: bool,
    failure: Option<Box<dyn Error>>,
}

impl App {
    pub(super) fn new(initial_rom: Option<PathBuf>) -> Self {
        let audio = AudioOutput::open()
            .inspect_err(|e| report(format_args!("Running without audio: {e}")))
            .ok();
        Self {
            initial_rom,
            display: None,
            title: format!("{WINDOW_TITLE} - Drag & Drop ROM here"),
            keys: Keys::default(),
            audio,
            audio_retry: None,
            session: None,
            clock: FrameClock::new(Instant::now()),
            discard: Discard::default(),
            unsaved: false,
            failure: None,
        }
    }

    /// Outcome once the event loop has ended, possibly with an error of its own.
    pub(super) fn into_result(
        self,
        loop_error: Option<Box<dyn Error>>,
    ) -> Result<ExitCode, Box<dyn Error>> {
        match self.failure.or(loop_error) {
            Some(error) if self.unsaved => {
                report(format_args!("RustyGB: {error}"));
                Ok(ExitCode::from(EXIT_UNSAVED))
            }
            Some(error) => Err(error),
            None if self.unsaved => Ok(ExitCode::from(EXIT_UNSAVED)),
            None => Ok(ExitCode::SUCCESS),
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: Box<dyn Error>) {
        self.failure = Some(error);
        event_loop.exit();
    }

    fn set_title(&mut self, text: &str) {
        self.title = format!("{WINDOW_TITLE} - {text}");
        if let Some(display) = &self.display {
            display.set_title(&self.title);
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop) {
        if self.persist() || self.discard_confirmed(Action::Close) {
            event_loop.exit();
        }
    }

    /// Returns whether no progress would be lost by dropping the session.
    fn persist(&mut self) -> bool {
        let saved = self.session.as_mut().is_none_or(Session::persist);
        if saved {
            self.discard.reset();
        }
        saved
    }

    /// Asks once, through the title, to repeat `action` before losing progress.
    fn discard_confirmed(&mut self, action: Action) -> bool {
        let verb = match action {
            Action::Close => "close",
            Action::Drop => "drop",
        };
        self.set_title(&format!("SAVE FAILED: {verb} again to lose progress"));
        self.discard.confirm(action, Instant::now())
    }

    /// Handles a key; focusing the window synthesizes a press of every held key.
    fn handle_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent, synthetic: bool) {
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        let pressed = event.state == ElementState::Pressed;
        // A held Escape must not confirm its own warning.
        if code == KeyCode::Escape && pressed && !event.repeat && !synthetic {
            self.close(event_loop);
        }
        self.keys.update(code, pressed);
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) {
        let frame = self.session.as_ref().map(|session| session.gb.frame());
        let Some(display) = &mut self.display else {
            return;
        };
        if let Err(e) = display.render(frame) {
            self.fail(event_loop, e);
        }
    }

    /// Emulates every frame that is due.
    fn advance(&mut self) {
        let mut redraw = false;
        // Bounded: the clock drops lag beyond a few frames.
        while self.clock.frame_due(Instant::now()) {
            redraw = true;
            self.run_frame();
        }
        if let (true, Some(display)) = (redraw, &self.display) {
            display.request_redraw();
        }
    }

    fn run_frame(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        for (button, pressed) in self.keys.next_frame() {
            session.gb.set_button(button, pressed);
        }
        session.run_frame();
        if let Some(audio) = &mut self.audio {
            audio.forward(&mut session.gb);
        }
    }

    /// Reopens the output after its device disappeared.
    fn recover_audio(&mut self) {
        let lost = self.audio.as_ref().is_some_and(AudioOutput::is_lost);
        let now = Instant::now();
        if lost {
            self.audio = None;
            self.audio_retry = Some(now);
        }
        if self.audio_retry.is_none_or(|retry| now < retry) {
            return;
        }
        self.audio = AudioOutput::open().ok();
        self.audio_retry = self.audio.is_none().then_some(now + AUDIO_RETRY_PERIOD);
        if let (Some(audio), Some(session)) = (&self.audio, &mut self.session) {
            session.gb.set_sample_rate(audio.sample_rate_hz());
        }
    }

    /// Replaces the running game, which stays in place if it cannot be saved (unless
    /// confirmed) or the new one cannot be opened. Saving comes first: the same ROM may be
    /// reopened.
    fn open_rom(&mut self, path: &Path) {
        if !self.persist() && !self.discard_confirmed(Action::Drop) {
            return;
        }
        let sample_rate_hz = self
            .audio
            .as_ref()
            .map_or(SILENT_SAMPLE_RATE_HZ, AudioOutput::sample_rate_hz);
        match Session::open(path, sample_rate_hz) {
            Ok(session) => {
                self.session = Some(session);
                self.discard.reset();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.set_title(&name);
            }
            Err(e) => {
                report(format_args!("Failed to load {}: {e}", path.display()));
                self.set_title(&e.to_string());
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.display.is_some() {
            return;
        }
        match Display::open(event_loop, &self.title) {
            Ok(display) => self.display = Some(display),
            Err(e) => return self.fail(event_loop, e),
        }
        if let Some(path) = self.initial_rom.take() {
            self.open_rom(&path);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.close(event_loop),
            WindowEvent::DroppedFile(path) => self.open_rom(&path),
            WindowEvent::Focused(false) => self.keys.release_all(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => self.handle_key(event_loop, &event, is_synthetic),
            WindowEvent::RedrawRequested => self.render(event_loop),
            WindowEvent::Resized(size) => {
                if let Some(display) = &mut self.display {
                    display.resize(size);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.recover_audio();
        self.advance();
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.clock.next_frame()));
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        // Not every exit goes through `close`: rendering may fail.
        self.unsaved = !self.persist();
    }
}
