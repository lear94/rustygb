//! Video: mode timing, interrupts and registers of the scanline renderer.
//!
//! Each line is drawn at once when H-Blank starts, so mid-line register changes are not
//! modelled.

mod render;
#[cfg(test)]
mod tests;

/// Visible width in pixels.
pub const SCREEN_WIDTH: usize = 160;
/// Visible height in pixels.
pub const SCREEN_HEIGHT: usize = 144;
/// Bytes per RGBA8 pixel.
const PIXEL_LEN: usize = 4;
const ROW_LEN: usize = SCREEN_WIDTH * PIXEL_LEN;

const DOTS_PER_TICK: u16 = 4;
const DOTS_PER_LINE: u16 = 456;
/// The line that enables the LCD starts partway.
const LCD_ON_FIRST_DOT: u16 = 4;
/// Dot at which a line enters its first mode.
const MODE_START_DOT: u16 = 4;
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
/// Value seen on the bus when nothing drives it.
const OPEN_BUS: u8 = 0xFF;

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
            Self::OamScan => STAT_OAM_SOURCE,
            Self::Drawing => 0,
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
    /// WY matched a line of this frame.
    window_triggered: bool,
    /// Lines of the window drawn this frame.
    window_line: u8,
    /// First dot of H-Blank on this line.
    drawing_end_dot: u16,
    mode: Mode,
    /// The first line after enabling the LCD has no OAM scan and reports mode 0 instead.
    skip_oam_scan: bool,
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
            lcdc: 0x91,
            stat_sources: 0,
            scy: 0,
            scx: 0,
            line: 0,
            ly: 0,
            lyc: 0,
            compared_line: Some(0),
            coincidence: true,
            wy: 0,
            wx: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            dots: 0,
            window_triggered: false,
            window_line: 0,
            drawing_end_dot: DRAWING_END_DOT,
            mode: Mode::OamScan,
            skip_oam_scan: false,
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
            if self.dots >= DOTS_PER_LINE {
                self.start_line();
            } else if self.dots == MODE_START_DOT {
                self.start_mode();
            } else if self.line < FIRST_VBLANK_LINE {
                self.update_visible_mode();
            } else if self.line == LAST_LINE && self.dots == LAST_LINE_WRAP_DOT {
                self.compare_lyc(Some(self.ly));
            }
        }
        std::mem::take(&mut self.requested)
    }

    /// Reads VRAM, OAM or a register; other addresses are open bus.
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x8000..=0x9FFF if self.vram_readable() => self.vram_byte(addr),
            0xFE00..=0xFE9F if self.oam_readable() => self
                .oam
                .get(usize::from(addr - OAM_BASE))
                .copied()
                .unwrap_or(OPEN_BUS),
            0xFF40 => self.lcdc,
            0xFF41 => self.stat(),
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            _ => OPEN_BUS,
        }
    }

    /// Writes VRAM, OAM or a register; other addresses are ignored.
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x8000..=0x9FFF if self.vram_writable() => {
                if let Some(cell) = self.vram.get_mut(usize::from(addr - VRAM_BASE)) {
                    *cell = val;
                }
            }
            0xFE00..=0xFE9F if self.oam_writable() => {
                self.write_oam(usize::from(addr - OAM_BASE), val);
            }
            0xFF40 => self.write_lcdc(val),
            0xFF41 => {
                // DMG quirk: for an instant every source is enabled.
                self.stat_sources = STAT_SOURCES;
                self.update_stat_line();
                self.stat_sources = val & STAT_SOURCES;
                self.update_stat_line();
            }
            0xFF42 => self.scy = val,
            0xFF43 => self.scx = val,
            0xFF45 => {
                self.lyc = val;
                self.compare_lyc(self.compared_line);
            }
            0xFF47 => self.bgp = val,
            0xFF48 => self.obp0 = val,
            0xFF49 => self.obp1 = val,
            0xFF4A => self.wy = val,
            0xFF4B => self.wx = val,
            _ => {}
        }
    }

    /// Writes an OAM byte, as DMA does; an index beyond OAM is ignored.
    pub fn write_oam(&mut self, idx: usize, val: u8) {
        if let Some(cell) = self.oam.get_mut(idx) {
            *cell = val;
        }
    }

    fn vram_readable(&self) -> bool {
        // Reads are cut off one M-cycle before writes.
        self.mode != Mode::Drawing && !self.at_scan_end()
    }

    fn vram_writable(&self) -> bool {
        self.mode != Mode::Drawing
    }

    fn oam_readable(&self) -> bool {
        // The scan claims OAM while STAT still reports mode 0.
        let scan_starting = self.dots == 0 && (1..FIRST_VBLANK_LINE).contains(&self.line);
        matches!(self.mode, Mode::HBlank | Mode::VBlank) && !scan_starting
    }

    fn oam_writable(&self) -> bool {
        // Between scan and drawing, writes get through for one M-cycle.
        matches!(self.mode, Mode::HBlank | Mode::VBlank) || self.at_scan_end()
    }

    fn at_scan_end(&self) -> bool {
        self.mode == Mode::OamScan && self.dots == OAM_SCAN_LAST_DOT
    }

    fn vram_byte(&self, addr: u16) -> u8 {
        self.vram
            .get(usize::from(addr.wrapping_sub(VRAM_BASE)))
            .copied()
            .unwrap_or(OPEN_BUS)
    }

    fn stat(&self) -> u8 {
        let coincidence = if self.coincidence {
            STAT_COINCIDENCE
        } else {
            0
        };
        STAT_UNUSED | self.stat_sources | coincidence | self.mode as u8
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
        self.update_stat_line();
        self.blank_frame();
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
            self.compare_lyc(None);
        }
    }

    /// Enters the first mode of the line, one M-cycle after LY changes.
    fn start_mode(&mut self) {
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
        // The OAM source also fires on this line, although nothing is scanned.
        self.mode = Mode::OamScan;
        self.update_stat_line();
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
        let lyc_source = if self.coincidence { STAT_LYC_SOURCE } else { 0 };
        let level = self.stat_sources & (mode_source | lyc_source) != 0;
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
