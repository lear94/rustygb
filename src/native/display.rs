//! Window and the pixel surface filling it.

use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
use rusty_gb::{SCREEN_HEIGHT, SCREEN_WIDTH};
use std::error::Error;
use std::sync::Arc;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

const SCREEN_WIDTH_PX: u32 = 160;
const SCREEN_HEIGHT_PX: u32 = 144;
const _: () = assert!(SCREEN_WIDTH_PX as usize == SCREEN_WIDTH);
const _: () = assert!(SCREEN_HEIGHT_PX as usize == SCREEN_HEIGHT);
/// Initial window magnification: 160x144 is tiny on current displays.
const WINDOW_SCALE: u32 = 3;

pub(super) struct Display {
    window: Arc<Window>,
    pixels: Pixels<'static>,
}

impl Display {
    pub(super) fn open(event_loop: &ActiveEventLoop, title: &str) -> Result<Self, Box<dyn Error>> {
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(
                SCREEN_WIDTH_PX * WINDOW_SCALE,
                SCREEN_HEIGHT_PX * WINDOW_SCALE,
            ))
            .with_min_inner_size(LogicalSize::new(SCREEN_WIDTH_PX, SCREEN_HEIGHT_PX));
        let window = Arc::new(event_loop.create_window(attributes)?);
        let size = window.inner_size();
        // Some platforms report no size until the first configuration.
        let surface =
            SurfaceTexture::new(size.width.max(1), size.height.max(1), Arc::clone(&window));
        // Waiting for the display would pace emulation and audio by its refresh rate.
        let pixels = PixelsBuilder::new(SCREEN_WIDTH_PX, SCREEN_HEIGHT_PX, surface)
            .enable_vsync(false)
            .build()?;
        Ok(Self { window, pixels })
    }

    pub(super) fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    pub(super) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(super) fn resize(&mut self, size: PhysicalSize<u32>) {
        if !is_visible(size) {
            return;
        }
        if let Err(e) = self.pixels.resize_surface(size.width, size.height) {
            eprintln!("Resize error: {e}");
        }
    }

    /// Presents `frame`, or black without one.
    pub(super) fn render(&mut self, frame: Option<&[u8]>) -> Result<(), pixels::Error> {
        if !is_visible(self.window.inner_size()) {
            return Ok(());
        }
        match frame {
            Some(frame) => self.pixels.frame_mut().copy_from_slice(frame),
            None => self.pixels.frame_mut().fill(0),
        }
        self.pixels.render()
    }
}

/// Minimising reports a zero size, which no surface can have.
fn is_visible(size: PhysicalSize<u32>) -> bool {
    size.width != 0 && size.height != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_visible_requires_both_dimensions() {
        for (width, height, expected) in [(0, 0, false), (0, 1, false), (1, 0, false), (1, 1, true)]
        {
            assert_eq!(is_visible(PhysicalSize::new(width, height)), expected);
        }
    }
}
