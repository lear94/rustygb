//! Video: mode timing, interrupts and registers of the scanline renderer.
//!
//! Each line is drawn at once when H-Blank starts, so mid-line register changes are not
//! modelled.

mod memory;
mod render;
#[cfg(test)]
mod tests;

use crate::hardware::T_CYCLES_PER_M_CYCLE;

/// Visible width in pixels.
pub const SCREEN_WIDTH: usize = 160;
/// Visible height in pixels.
pub const SCREEN_HEIGHT: usize = 144;
/// Bytes per RGBA8 pixel.
const PIXEL_LEN: usize = 4;
const ROW_LEN: usize = SCREEN_WIDTH * PIXEL_LEN;

/// One dot per T-cycle.
const DOTS_PER_TICK: u16 = T_CYCLES_PER_M_CYCLE;
const DOTS_PER_LINE: u16 = 456;
/// The line that enables the LCD starts partway.
const LCD_ON_FIRST_DOT: u16 = 4;
/// Dot at which a line enters its first mode.
const MODE_START_DOT: u16 = 4;
/// Dot of the last line at which the boot ROM hands over.
const POST_BOOT_DOT: u16 = 396;
/// Dot at which line 0 ends its OAM pulse and line 153 its LYC match.
const PULSE_END_DOT: u16 = 8;
/// Dot of the last line at which LYC sees line 0.
const LAST_LINE_WRAP_DOT: u16 = 12;
/// Last dot sampled as mode 2 and first as mode 0, on the M-cycle grid.
const OAM_SCAN_LAST_DOT: u16 = 80;
const DRAWING_END_DOT: u16 = 256;
const FIRST_VBLANK_LINE: u8 = 144;
const LAST_LINE: u8 = 153;

const VRAM_LEN: usize = 8 * 1024;
const VRAM_BASE: u16 = 0x8000;
const OAM_LEN: usize = 160;
const OAM_BASE: u16 = 0xFE00;

const POST_BOOT_LCDC: u8 = 0x91;
const POST_BOOT_BGP: u8 = 0xFC;
const POST_BOOT_OBP: u8 = 0xFF;
const LCDC_ON: u8 = 0x80;
const STAT_LYC_SOURCE: u8 = 0x40;
const STAT_OAM_SOURCE: u8 = 0x20;
const STAT_VBLANK_SOURCE: u8 = 0x10;
const STAT_HBLANK_SOURCE: u8 = 0x08;
const STAT_COINCIDENCE: u8 = 0x04;
const STAT_SOURCES: u8 = 0x78;
/// The STAT bit that does not exist reads as 1.
const STAT_UNUSED: u8 = 0x80;

const IRQ_VBLANK: u8 = 0x01;
const IRQ_STAT: u8 = 0x02;

/// PPU mode, numbered as STAT reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    HBlank = 0,
    VBlank = 1,
    OamScan = 2,
    Drawing = 3,
}

impl Mode {
    /// STAT bit enabling the interrupt of this mode.
    fn stat_source(self) -> u8 {
        match self {
            Self::HBlank => STAT_HBLANK_SOURCE,
            Self::VBlank => STAT_VBLANK_SOURCE,
            // The OAM source is a pulse at the line start, not the whole scan.
            Self::OamScan | Self::Drawing => 0,
        }
    }
}

/// Pixel processing unit.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent one-bit hardware latches"
)]
pub struct Ppu {
    /// Last complete picture.
    frame: Vec<u8>,
    /// Picture being drawn.
    back: Vec<u8>,
    /// The first picture after enabling the LCD is never shown.
    skip_frame: bool,
    vram: [u8; VRAM_LEN],
    oam: [u8; OAM_LEN],
    lcdc: u8,
    stat_sources: u8,
    scy: u8,
    scx: u8,
    /// Scanline, 0-153.
    line: u8,
    /// Scanline as reported, which restarts early.
    ly: u8,
    lyc: u8,
    /// Scanline LYC is compared with.
    compared_line: Option<u8>,
    /// Frozen while the LCD is off.
    coincidence: bool,
    wy: u8,
    wx: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    dots: u16,
    /// Dot of the next state change on this line.
    next_event_dot: u16,
    /// WY matched a line of this frame.
    window_triggered: bool,
    /// Lines of the window drawn this frame.
    window_line: u8,
    /// First dot of H-Blank on this line.
    drawing_end_dot: u16,
    mode: Mode,
    /// The first line after enabling the LCD has no OAM scan and reports mode 0 instead.
    skip_oam_scan: bool,
    /// The OAM source is being pulsed.
    oam_pulse: bool,
    /// OR of all STAT sources; only its rising edge interrupts.
    stat_line: bool,
    requested: u8,
}

impl Ppu {
    /// Creates a PPU in its post-boot state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            frame: vec![0; ROW_LEN * SCREEN_HEIGHT],
            back: vec![0; ROW_LEN * SCREEN_HEIGHT],
            skip_frame: false,
            vram: [0; VRAM_LEN],
            oam: [0; OAM_LEN],
            lcdc: POST_BOOT_LCDC,
            stat_sources: 0,
            scy: 0,
            scx: 0,
            line: LAST_LINE,
            ly: 0,
            lyc: 0,
            compared_line: Some(0),
            coincidence: true,
            wy: 0,
            wx: 0,
            bgp: POST_BOOT_BGP,
            obp0: POST_BOOT_OBP,
            obp1: POST_BOOT_OBP,
            dots: POST_BOOT_DOT,
            next_event_dot: DOTS_PER_LINE,
            window_triggered: false,
            window_line: 0,
            drawing_end_dot: DRAWING_END_DOT,
            mode: Mode::VBlank,
            skip_oam_scan: false,
            oam_pulse: false,
            stat_line: false,
            requested: 0,
        }
    }

    /// Borrows the 160x144 RGBA8 frame, row by row from the top left.
    #[must_use]
    pub fn frame(&self) -> &[u8] {
        &self.frame
    }

    /// Advances one M-cycle.
    ///
    /// # Returns
    /// Requested interrupts as IF bits.
    ///
    /// # Complexity
    /// O(1), bounded by one 160-pixel line; no allocation.
    pub fn tick(&mut self) -> u8 {
        if self.lcdc & LCDC_ON != 0 {
            self.dots += DOTS_PER_TICK;
            if self.dots >= self.next_event_dot {
                self.handle_event();
            }
        }
        std::mem::take(&mut self.requested)
    }

    fn write_lcdc(&mut self, val: u8) {
        let was_on = self.lcdc & LCDC_ON != 0;
        let is_on = val & LCDC_ON != 0;
        self.lcdc = val;
        if was_on && !is_on {
            self.turn_off();
        }
        if !was_on && is_on {
            self.skip_oam_scan = true;
            self.dots = LCD_ON_FIRST_DOT;
            self.next_event_dot = self.following_event_dot();
            self.skip_frame = true;
            self.compare_lyc(Some(self.ly));
        }
    }

    fn turn_off(&mut self) {
        self.line = 0;
        self.ly = 0;
        self.dots = 0;
        self.window_line = 0;
        self.window_triggered = false;
        self.mode = Mode::HBlank;
        self.oam_pulse = false;
        self.update_stat_line();
        self.blank_frame();
    }

    /// Returns whether the last tick changed the scanline.
    pub(crate) fn at_line_start(&self) -> bool {
        self.dots == 0
    }

    /// Runs the state change due at this dot and schedules the next one. Most ticks have
    /// none, which keeps [`Self::tick`] to one comparison.
    fn handle_event(&mut self) {
        if self.dots >= DOTS_PER_LINE {
            self.start_line();
        } else if self.dots == MODE_START_DOT {
            self.start_mode();
        } else if self.dots == PULSE_END_DOT {
            self.end_pulse();
        } else if self.line < FIRST_VBLANK_LINE {
            self.update_visible_mode();
        } else if self.line == LAST_LINE && self.dots == LAST_LINE_WRAP_DOT {
            self.compare_lyc(Some(self.ly));
        }
        self.next_event_dot = self.following_event_dot();
    }

    fn following_event_dot(&self) -> u16 {
        let visible = self.line < FIRST_VBLANK_LINE;
        if self.dots < MODE_START_DOT {
            MODE_START_DOT
        } else if self.dots < PULSE_END_DOT && matches!(self.line, 0 | LAST_LINE) {
            PULSE_END_DOT
        } else if self.line == LAST_LINE && self.dots < LAST_LINE_WRAP_DOT {
            LAST_LINE_WRAP_DOT
        } else if visible && self.dots <= OAM_SCAN_LAST_DOT {
            OAM_SCAN_LAST_DOT + DOTS_PER_TICK
        } else if visible && self.mode == Mode::Drawing {
            self.drawing_end_dot
        } else {
            DOTS_PER_LINE
        }
    }

    fn start_line(&mut self) {
        self.dots = 0;
        self.line = if self.line == LAST_LINE {
            0
        } else {
            self.line + 1
        };
        self.ly = self.line;
        if self.line != 0 {
            self.oam_pulse = self.line <= FIRST_VBLANK_LINE;
            self.compare_lyc(None);
        }
    }

    /// Ends what the first and last lines hold for one more M-cycle than the others.
    fn end_pulse(&mut self) {
        if self.line == 0 {
            self.oam_pulse = false;
            self.update_stat_line();
        } else {
            self.compare_lyc(None);
        }
    }

    /// Enters the first mode of the line, one M-cycle after LY changes.
    fn start_mode(&mut self) {
        self.oam_pulse = self.line == 0 && !self.skip_oam_scan;
        if self.line == FIRST_VBLANK_LINE {
            self.enter_vblank();
        } else if self.line < FIRST_VBLANK_LINE && !self.skip_oam_scan {
            self.mode = Mode::OamScan;
        }
        // Mode and coincidence change together, so the STAT line sees a single edge.
        self.compare_lyc(Some(self.line));
        if self.line == LAST_LINE {
            // LY stops showing the last line long before it ends.
            self.ly = 0;
        }
    }

    fn enter_vblank(&mut self) {
        self.requested |= IRQ_VBLANK;
        self.mode = Mode::VBlank;
        self.window_line = 0;
        self.window_triggered = false;
        if std::mem::take(&mut self.skip_frame) {
            return;
        }
        std::mem::swap(&mut self.frame, &mut self.back);
    }

    fn update_visible_mode(&mut self) {
        if self.dots <= OAM_SCAN_LAST_DOT {
            return;
        }
        let mode = if self.mode == Mode::OamScan || self.skip_oam_scan {
            self.drawing_end_dot = DRAWING_END_DOT + self.drawing_penalty();
            Mode::Drawing
        } else if self.dots < self.drawing_end_dot {
            return;
        } else {
            Mode::HBlank
        };
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.skip_oam_scan = false;
        self.update_stat_line();
        if mode == Mode::HBlank {
            self.render_line();
        }
    }

    /// Compares LYC with the scanline, or with nothing while the scanline changes.
    fn compare_lyc(&mut self, line: Option<u8>) {
        self.compared_line = line;
        if self.lcdc & LCDC_ON != 0 {
            self.coincidence = line == Some(self.lyc);
            self.update_stat_line();
        }
    }

    fn update_stat_line(&mut self) {
        let mode_source = if self.lcdc & LCDC_ON != 0 && !self.skip_oam_scan {
            self.mode.stat_source()
        } else {
            0
        };
        let oam_source = if self.oam_pulse { STAT_OAM_SOURCE } else { 0 };
        let lyc_source = if self.coincidence { STAT_LYC_SOURCE } else { 0 };
        let level = self.stat_sources & (mode_source | oam_source | lyc_source) != 0;
        if level && !self.stat_line {
            self.requested |= IRQ_STAT;
        }
        self.stat_line = level;
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}
