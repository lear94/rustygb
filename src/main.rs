//! Native desktop front-end for the RustyGB core.
//!
//! Opens a window, wires keyboard input to [`rusty_gb::Button`], pumps the
//! [`rusty_gb::GameBoy`] one frame per display refresh, and forwards APU
//! samples to the default audio output through `cpal`.
//!
//! On `wasm32-*` targets the binary is compiled as an empty stub; the
//! WebAssembly front-end uses [`rusty_gb::wasm`] directly.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use pixels::{Pixels, SurfaceTexture};
    use rusty_gb::{Button, GameBoy, SCREEN_HEIGHT, SCREEN_WIDTH};
    use std::env;
    use std::path::Path;
    use std::thread;
    use std::time::{Duration, Instant};
    use winit::dpi::LogicalSize;
    use winit::event::{Event, VirtualKeyCode, WindowEvent};
    use winit::event_loop::{ControlFlow, EventLoop};
    use winit::window::WindowBuilder;
    use winit_input_helper::WinitInputHelper;

    /// Frame budget for a 59.73 Hz DMG (~16.743 ms).
    const FRAME_DURATION: Duration = Duration::from_micros(16_743);

    /// Capacity of the audio bridge between the emulator thread and the
    /// `cpal` output callback. Roughly 170 ms at 48 kHz.
    const AUDIO_CHANNEL_CAPACITY: usize = 8192;

    pub fn run() {
        let event_loop = EventLoop::new();
        let mut input = WinitInputHelper::new();

        let window = WindowBuilder::new()
            .with_title("RustyGB - Drag & Drop ROM here")
            .with_inner_size(LogicalSize::new(
                SCREEN_WIDTH as f64 * 3.0,
                SCREEN_HEIGHT as f64 * 3.0,
            ))
            .with_min_inner_size(LogicalSize::new(SCREEN_WIDTH as f64, SCREEN_HEIGHT as f64))
            .build(&event_loop)
            .unwrap();

        let mut pixels = {
            let size = window.inner_size();
            let surface = SurfaceTexture::new(size.width, size.height, &window);
            Pixels::new(SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32, surface).unwrap()
        };

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("No audio output device available");
        let config = device.default_output_config().unwrap();
        let sample_rate = config.sample_rate().0;

        let (audio_tx, audio_rx) = crossbeam_channel::bounded::<f32>(AUDIO_CHANNEL_CAPACITY);

        let stream = device
            .build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for sample in data.iter_mut() {
                        *sample = audio_rx.try_recv().unwrap_or(0.0) * 0.1;
                    }
                },
                |e| eprintln!("Audio stream error: {}", e),
                None,
            )
            .unwrap();
        stream.play().unwrap();

        let mut gb: Option<GameBoy> = None;
        let mut rom_path: Option<String> = None;
        let mut audio_buffer = vec![0.0f32; 2048];

        let args: Vec<String> = env::args().collect();
        if let Some(path) = args.get(1) {
            match load_rom(path, sample_rate) {
                Ok(g) => {
                    gb = Some(g);
                    rom_path = Some(path.clone());
                    window.set_title("RustyGB - Running from CLI");
                }
                Err(e) => eprintln!("Error loading ROM: {}", e),
            }
        }

        let mut last_frame_time = Instant::now();

        event_loop.run(move |event, _, control_flow| {
            if let Event::WindowEvent {
                event: WindowEvent::DroppedFile(path),
                ..
            } = &event
            {
                let path_str = path.to_string_lossy().to_string();
                println!("File dropped: {}", path_str);
                match load_rom(&path_str, sample_rate) {
                    Ok(g) => {
                        gb = Some(g);
                        rom_path = Some(path_str.clone());
                        window.set_title(&format!(
                            "RustyGB - Playing: {}",
                            path.file_name().unwrap().to_string_lossy()
                        ));
                    }
                    Err(e) => eprintln!("Failed to load dropped ROM: {}", e),
                }
            }

            if let Event::LoopDestroyed = event {
                if let (Some(g), Some(path)) = (gb.as_ref(), rom_path.as_ref()) {
                    println!("Saving game to {}...", path);
                    let _ = g.save_external_ram(path);
                }
                return;
            }

            if input.update(&event) {
                if input.key_pressed(VirtualKeyCode::Escape) || input.close_requested() {
                    *control_flow = ControlFlow::Exit;
                    return;
                }
                if let Some(g) = gb.as_mut() {
                    apply_input(g, &input);
                }
                if let Some(size) = input.window_resized() {
                    pixels.resize_surface(size.width, size.height).unwrap();
                }
            }

            if let Event::RedrawRequested(_) = event {
                if let Some(g) = gb.as_ref() {
                    pixels.frame_mut().copy_from_slice(g.frame());
                } else {
                    pixels.frame_mut().fill(0);
                }
                if let Err(e) = pixels.render() {
                    eprintln!("Render error: {}", e);
                    *control_flow = ControlFlow::Exit;
                }
            }

            if let Event::MainEventsCleared = event {
                let elapsed = last_frame_time.elapsed();
                if elapsed < FRAME_DURATION {
                    thread::yield_now();
                    return;
                }
                last_frame_time = Instant::now();

                if let Some(g) = gb.as_mut() {
                    g.run_frame();
                    forward_audio(g, &audio_tx, &mut audio_buffer);
                }
                window.request_redraw();
            }
        });
    }

    /// Drain the APU's pending samples into the `cpal` ring buffer until
    /// either the APU runs dry or the consumer can no longer keep up.
    fn forward_audio(gb: &mut GameBoy, tx: &crossbeam_channel::Sender<f32>, scratch: &mut [f32]) {
        loop {
            let n = gb.drain_audio(scratch);
            if n == 0 {
                return;
            }
            for &sample in &scratch[..n] {
                if tx.try_send(sample).is_err() {
                    return;
                }
            }
            if n < scratch.len() {
                return;
            }
        }
    }

    /// Build a [`GameBoy`] from a ROM on disk, loading any `.sav` sibling
    /// as the initial cartridge RAM.
    fn load_rom(path: &str, sample_rate: u32) -> anyhow::Result<GameBoy> {
        let rom = std::fs::read(path)?;
        let save_path = Path::new(path).with_extension("sav");
        let saved_ram = if save_path.exists() {
            std::fs::read(save_path).ok()
        } else {
            None
        };
        GameBoy::from_rom_with_save(rom, saved_ram, sample_rate)
    }

    /// Push current keyboard state into the emulated joypad.
    ///
    /// Default key mapping:
    ///
    /// | Game Boy | Key       |
    /// |----------|-----------|
    /// | A        | `Z`       |
    /// | B        | `X`       |
    /// | Select   | `Space`   |
    /// | Start    | `Enter`   |
    /// | D-Pad    | Arrows    |
    fn apply_input(gb: &mut GameBoy, input: &WinitInputHelper) {
        const MAPPING: &[(VirtualKeyCode, Button)] = &[
            (VirtualKeyCode::Z, Button::A),
            (VirtualKeyCode::X, Button::B),
            (VirtualKeyCode::Space, Button::Select),
            (VirtualKeyCode::Return, Button::Start),
            (VirtualKeyCode::Right, Button::Right),
            (VirtualKeyCode::Left, Button::Left),
            (VirtualKeyCode::Up, Button::Up),
            (VirtualKeyCode::Down, Button::Down),
        ];
        for &(key, button) in MAPPING {
            gb.set_button(button, input.key_held(key));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::run();
}

#[cfg(target_arch = "wasm32")]
fn main() {}
