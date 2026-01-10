mod apu;
mod bus;
mod cartridge;
mod cpu;
mod joypad;
mod ppu;
mod timer;

use crate::bus::Motherboard;
use crate::cartridge::load_cartridge;
use crate::cpu::Cpu;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use pixels::{Pixels, SurfaceTexture};
use std::env;
use std::thread;
use std::time::{Duration, Instant};
use winit::dpi::LogicalSize;
use winit::event::{Event, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;
use winit_input_helper::WinitInputHelper;

const SCREEN_WIDTH: u32 = 160;
const SCREEN_HEIGHT: u32 = 144;
const CYCLES_PER_FRAME: u32 = 70224;
const FRAME_DURATION: Duration = Duration::from_micros(16743);

fn main() {
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
        Pixels::new(SCREEN_WIDTH, SCREEN_HEIGHT, surface).unwrap()
    };

    // Audio Setup
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("No audio output device available");
    let config = device.default_output_config().unwrap();
    let sample_rate = config.sample_rate().0;

    // Emulator State
    let mut bus: Option<Motherboard> = None;
    let mut cpu: Option<Cpu> = None;
    let mut rom_path_cache: Option<String> = None;

    // Audio Channel
    let (tx, rx) = crossbeam_channel::bounded(4096);

    let _stream = device
        .build_output_stream(
            &config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for sample in data.iter_mut() {
                    *sample = rx.try_recv().unwrap_or(0.0) * 0.1;
                }
            },
            |e| eprintln!("Audio stream error: {}", e),
            None,
        )
        .unwrap();
    _stream.play().unwrap();

    // Load ROM from arguments if present
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        let path = args[1].clone();
        match load_system(&path, sample_rate, tx.clone()) {
            Ok((c, m)) => {
                cpu = Some(c);
                bus = Some(m);
                rom_path_cache = Some(path);
                window.set_title("RustyGB - Running from CLI");
            }
            Err(e) => eprintln!("Error loading ROM: {}", e),
        }
    }

    let mut last_frame_time = Instant::now();

    // --- MAIN LOOP ---
    event_loop.run(move |event, _, control_flow| {
        // 1. Drag & Drop Handling
        if let Event::WindowEvent {
            event: WindowEvent::DroppedFile(path),
            ..
        } = &event
        {
            let path_str = path.to_string_lossy().to_string();
            println!("File Dropped: {}", path_str);
            match load_system(&path_str, sample_rate, tx.clone()) {
                Ok((c, m)) => {
                    cpu = Some(c);
                    bus = Some(m);
                    rom_path_cache = Some(path_str.clone());
                    window.set_title(&format!(
                        "RustyGB - Playing: {}",
                        path.file_name().unwrap().to_string_lossy()
                    ));
                }
                Err(e) => eprintln!("Failed to load dropped ROM: {}", e),
            }
        }

        // 2. Persistence on Close
        if let Event::LoopDestroyed = event {
            if let Some(ref b) = bus {
                if let Some(ref path) = rom_path_cache {
                    println!("Saving game to {}...", path);
                    let _ = b.save_external_ram(path);
                }
            }
            return;
        }

        // 3. Input and Window Resizing
        if input.update(&event) {
            if input.key_pressed(VirtualKeyCode::Escape) || input.close_requested() {
                *control_flow = ControlFlow::Exit;
                return;
            }

            if let Some(ref mut b) = bus {
                b.joypad.update(&input);
            }

            if let Some(size) = input.window_resized() {
                pixels.resize_surface(size.width, size.height).unwrap();
            }
        }

        // 4. Rendering
        if let Event::RedrawRequested(_) = event {
            if let Some(ref mut b) = bus {
                pixels.frame_mut().copy_from_slice(&b.ppu.buffer);
            } else {
                pixels.frame_mut().fill(0);
            }

            if let Err(e) = pixels.render() {
                eprintln!("Render error: {}", e);
                *control_flow = ControlFlow::Exit;
            }
        }

        // 5. Emulation (Core Loop)
        match event {
            Event::MainEventsCleared => {
                let elapsed = last_frame_time.elapsed();
                if elapsed < FRAME_DURATION {
                    thread::yield_now();
                    return;
                }
                last_frame_time = Instant::now();

                if let (Some(ref mut c), Some(ref mut b)) = (&mut cpu, &mut bus) {
                    let mut cycles = 0;
                    while cycles < CYCLES_PER_FRAME {
                        let step_cycles = c.step(b);
                        b.tick(step_cycles);
                        cycles += step_cycles as u32;
                    }
                }

                window.request_redraw();
            }
            _ => (),
        }
    });
}

fn load_system(
    path: &str,
    sample_rate: u32,
    tx: crossbeam_channel::Sender<f32>,
) -> anyhow::Result<(Cpu, Motherboard)> {
    let cart = load_cartridge(path)?;
    let mut bus = Motherboard::new(cart, sample_rate);
    bus.apu.sender = Some(tx);
    let mut cpu = Cpu::new();
    cpu.reset_to_boot();
    Ok((cpu, bus))
}
