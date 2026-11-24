//! Desktop front-end: window, keyboard, frame pacing and audio around the core.

mod audio;
mod frame_clock;
mod session;

use audio::AudioOutput;
use frame_clock::FrameClock;
use pixels::{Pixels, SurfaceTexture};
use rusty_gb::{Button, GameBoy, SCREEN_HEIGHT, SCREEN_WIDTH};
use session::Session;
use std::convert::Infallible;
use std::env;
use std::error::Error;
use std::path::Path;
use std::time::Instant;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{Event, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};
use winit_input_helper::WinitInputHelper;

const SCREEN_WIDTH_PX: u32 = 160;
const SCREEN_HEIGHT_PX: u32 = 144;
const _: () = assert!(SCREEN_WIDTH_PX as usize == SCREEN_WIDTH);
const _: () = assert!(SCREEN_HEIGHT_PX as usize == SCREEN_HEIGHT);
/// Initial window magnification: 160x144 is tiny on current displays.
const WINDOW_SCALE: u32 = 3;
/// Core output rate while no audio device consumes it.
const SILENT_SAMPLE_RATE_HZ: u32 = 48_000;

const KEY_MAP: [(VirtualKeyCode, Button); 8] = [
    (VirtualKeyCode::Z, Button::A),
    (VirtualKeyCode::X, Button::B),
    (VirtualKeyCode::Space, Button::Select),
    (VirtualKeyCode::Return, Button::Start),
    (VirtualKeyCode::Right, Button::Right),
    (VirtualKeyCode::Left, Button::Left),
    (VirtualKeyCode::Up, Button::Up),
    (VirtualKeyCode::Down, Button::Down),
];

struct App {
    window: Window,
    pixels: Pixels,
    input: WinitInputHelper,
    audio: Option<AudioOutput>,
    session: Option<Session>,
    clock: FrameClock,
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
        let pixels = Pixels::new(SCREEN_WIDTH_PX, SCREEN_HEIGHT_PX, surface)?;
        let audio = AudioOutput::open()
            .inspect_err(|e| eprintln!("Running without audio: {e}"))
            .ok();
        Ok(Self {
            window,
            pixels,
            input: WinitInputHelper::new(),
            audio,
            session: None,
            clock: FrameClock::new(Instant::now()),
        })
    }

    fn handle(&mut self, event: &Event<'_, ()>, control_flow: &mut ControlFlow) {
        match event {
            Event::WindowEvent {
                event: WindowEvent::DroppedFile(path),
                ..
            } => self.open_rom(path),
            Event::RedrawRequested(_) => {
                if let Err(e) = self.render() {
                    eprintln!("Render error: {e}");
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::LoopDestroyed => {
                if let Some(session) = self.session.as_mut() {
                    session.persist();
                }
            }
            _ => {}
        }
        // The helper reports a finished event batch on `MainEventsCleared`.
        if self.input.update(event) {
            self.advance(control_flow);
        }
    }

    fn advance(&mut self, control_flow: &mut ControlFlow) {
        if self.input.key_pressed(VirtualKeyCode::Escape) || self.input.close_requested() {
            *control_flow = ControlFlow::Exit;
            return;
        }
        if let Some(size) = self.input.window_resized() {
            self.resize(size);
        }
        if self.clock.frame_due(Instant::now()) {
            if let Some(session) = self.session.as_mut() {
                apply_input(&mut session.gb, &self.input);
                session.run_frame();
                if let Some(audio) = self.audio.as_mut() {
                    audio.forward(&mut session.gb);
                }
            }
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
        self.pixels.render()
    }

    /// Replaces the running game, which stays in place if it cannot be saved or the new
    /// one cannot be opened. Saving comes first: the same ROM may be reopened.
    fn open_rom(&mut self, path: &Path) {
        if !self.session.as_mut().is_none_or(Session::persist) {
            return;
        }
        let sample_rate_hz = self
            .audio
            .as_ref()
            .map_or(SILENT_SAMPLE_RATE_HZ, AudioOutput::sample_rate_hz);
        match Session::open(path, sample_rate_hz) {
            Ok(session) => {
                self.session = Some(session);
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.window.set_title(&format!("RustyGB - {name}"));
            }
            Err(e) => eprintln!("Failed to load {}: {e}", path.display()),
        }
    }
}

fn apply_input(gb: &mut GameBoy, input: &WinitInputHelper) {
    for (key, button) in KEY_MAP {
        gb.set_button(button, input.key_held(key));
    }
}
