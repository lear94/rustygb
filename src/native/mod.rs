//! Desktop front-end: window, keyboard, frame pacing and audio around the core.

mod audio;
mod frame_clock;
mod keys;
mod session;

use audio::AudioOutput;
use frame_clock::FrameClock;
use keys::Keys;
use pixels::wgpu::SurfaceError;
use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
use rusty_gb::{SCREEN_HEIGHT, SCREEN_WIDTH};
use session::Session;
use std::convert::Infallible;
use std::env;
use std::error::Error;
use std::path::Path;
use std::time::Instant;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, Event, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};

const SCREEN_WIDTH_PX: u32 = 160;
const SCREEN_HEIGHT_PX: u32 = 144;
const _: () = assert!(SCREEN_WIDTH_PX as usize == SCREEN_WIDTH);
const _: () = assert!(SCREEN_HEIGHT_PX as usize == SCREEN_HEIGHT);
/// Initial window magnification: 160x144 is tiny on current displays.
const WINDOW_SCALE: u32 = 3;
/// Core output rate while no audio device consumes it.
const SILENT_SAMPLE_RATE_HZ: u32 = 48_000;

/// Exit code of a session closed with unsaved progress.
const EXIT_UNSAVED: i32 = 2;

struct App {
    window: Window,
    pixels: Pixels,
    keys: Keys,
    audio: Option<AudioOutput>,
    session: Option<Session>,
    clock: FrameClock,
    /// The user was told that repeating the action loses progress.
    discard_armed: bool,
}

/// Runs until the window closes, which ends the process; returns only on failure.
pub(crate) fn run() -> Result<Infallible, Box<dyn Error>> {
    let event_loop = EventLoop::new();
    let mut app = App::new(&event_loop)?;
    if let Some(path) = env::args_os().nth(1) {
        app.open_rom(Path::new(&path));
    }
    event_loop.run(move |event, _, control_flow| app.handle(&event, control_flow))
}

impl App {
    fn new(event_loop: &EventLoop<()>) -> Result<Self, Box<dyn Error>> {
        let window = WindowBuilder::new()
            .with_title("RustyGB - Drag & Drop ROM here")
            .with_inner_size(LogicalSize::new(
                SCREEN_WIDTH_PX * WINDOW_SCALE,
                SCREEN_HEIGHT_PX * WINDOW_SCALE,
            ))
            .with_min_inner_size(LogicalSize::new(SCREEN_WIDTH_PX, SCREEN_HEIGHT_PX))
            .build(event_loop)?;
        let size = window.inner_size();
        let surface = SurfaceTexture::new(size.width, size.height, &window);
        // Waiting for the display would pace emulation and audio by its refresh rate.
        let pixels = PixelsBuilder::new(SCREEN_WIDTH_PX, SCREEN_HEIGHT_PX, surface)
            .enable_vsync(false)
            .build()?;
        let audio = AudioOutput::open()
            .inspect_err(|e| eprintln!("Running without audio: {e}"))
            .ok();
        Ok(Self {
            window,
            pixels,
            keys: Keys::default(),
            audio,
            session: None,
            clock: FrameClock::new(Instant::now()),
            discard_armed: false,
        })
    }

    fn handle(&mut self, event: &Event<'_, ()>, control_flow: &mut ControlFlow) {
        match event {
            Event::WindowEvent { event, .. } => self.handle_window(event, control_flow),
            Event::MainEventsCleared => self.advance(control_flow),
            Event::RedrawRequested(_) => {
                if let Err(e) = self.render() {
                    eprintln!("Render error: {e}");
                    *control_flow = ControlFlow::Exit;
                }
            }
            // Not every exit goes through `close`: rendering may fail.
            Event::LoopDestroyed => {
                self.persist();
            }
            _ => {}
        }
    }

    fn handle_window(&mut self, event: &WindowEvent<'_>, control_flow: &mut ControlFlow) {
        match event {
            WindowEvent::CloseRequested => self.close(control_flow),
            WindowEvent::DroppedFile(path) => self.open_rom(path),
            WindowEvent::Resized(size) => self.resize(*size),
            WindowEvent::Focused(false) => self.keys.release_all(),
            WindowEvent::KeyboardInput { input, .. } => {
                let escape = input.virtual_keycode == Some(VirtualKeyCode::Escape);
                if escape && input.state == ElementState::Pressed {
                    self.close(control_flow);
                }
                self.keys.update(*input);
            }
            _ => {}
        }
    }

    fn close(&mut self, control_flow: &mut ControlFlow) {
        if self.persist() {
            *control_flow = ControlFlow::Exit;
        } else if self.discard_confirmed("close") {
            *control_flow = ControlFlow::ExitWithCode(EXIT_UNSAVED);
        }
    }

    /// Returns whether no progress would be lost by dropping the session.
    fn persist(&mut self) -> bool {
        let saved = self.session.as_mut().is_none_or(Session::persist);
        if saved {
            self.discard_armed = false;
        }
        saved
    }

    /// Asks once, through the title, to repeat `action` before losing progress.
    fn discard_confirmed(&mut self, action: &str) -> bool {
        self.window.set_title(&format!(
            "RustyGB - SAVE FAILED: {action} again to lose progress"
        ));
        std::mem::replace(&mut self.discard_armed, true)
    }

    fn advance(&mut self, control_flow: &mut ControlFlow) {
        if matches!(control_flow, ControlFlow::ExitWithCode(_)) {
            return;
        }
        let mut redraw = false;
        // Bounded: the clock drops lag beyond a few frames.
        while self.clock.frame_due(Instant::now()) {
            redraw = true;
            if let Some(session) = self.session.as_mut() {
                for (button, pressed) in self.keys.next_frame() {
                    session.gb.set_button(button, pressed);
                }
                session.run_frame();
                if let Some(audio) = self.audio.as_mut() {
                    audio.forward(&mut session.gb);
                }
            }
        }
        if redraw {
            self.window.request_redraw();
        }
        *control_flow = ControlFlow::WaitUntil(self.clock.next_frame());
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        // Minimising reports a zero size, which no surface can have.
        if size.width == 0 || size.height == 0 {
            return;
        }
        if let Err(e) = self.pixels.resize_surface(size.width, size.height) {
            eprintln!("Resize error: {e}");
        }
    }

    fn render(&mut self) -> Result<(), pixels::Error> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        match self.session.as_ref() {
            Some(session) => self.pixels.frame_mut().copy_from_slice(session.gb.frame()),
            None => self.pixels.frame_mut().fill(0),
        }
        match self.pixels.render() {
            // An occluded window presents nothing; the next frame tries again.
            Err(pixels::Error::Surface(SurfaceError::Timeout)) => Ok(()),
            result => result,
        }
    }

    /// Replaces the running game, which stays in place if it cannot be saved (unless
    /// confirmed) or the new one cannot be opened. Saving comes first: the same ROM may be
    /// reopened.
    fn open_rom(&mut self, path: &Path) {
        if !self.persist() && !self.discard_confirmed("drop") {
            return;
        }
        let sample_rate_hz = self
            .audio
            .as_ref()
            .map_or(SILENT_SAMPLE_RATE_HZ, AudioOutput::sample_rate_hz);
        match Session::open(path, sample_rate_hz) {
            Ok(session) => {
                self.session = Some(session);
                self.discard_armed = false;
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.window.set_title(&format!("RustyGB - {name}"));
            }
            Err(e) => {
                eprintln!("Failed to load {}: {e}", path.display());
                self.window.set_title(&format!("RustyGB - {e}"));
            }
        }
    }
}
