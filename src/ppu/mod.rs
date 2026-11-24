//! Video: mode timing, interrupts and registers of the scanline renderer.
//!
//! Each line is drawn at once when H-Blank starts, so mid-line register changes, the
//! variable length of mode 3 and VRAM/OAM blocking are not modelled.

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
/// Last dot sampled as mode 2, then as mode 3, on the M-cycle grid.
const OAM_SCAN_LAST_DOT: u16 = 80;
const DRAWING_LAST_DOT: u16 = 252;
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
pub struct Ppu {
    frame: Vec<u8>,
    vram: [u8; VRAM_LEN],
    oam: [u8; OAM_LEN],
    lcdc: u8,
    stat_sources: u8,
    scy: u8,
    scx: u8,
    ly: u8,
    lyc: u8,
    /// Frozen while the LCD is off.
    coincidence: bool,
    wy: u8,
    wx: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    dots: u16,
    /// Lines of the window drawn this frame.
    window_line: u8,
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
            vram: [0; VRAM_LEN],
            oam: [0; OAM_LEN],
            lcdc: 0x91,
            stat_sources: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            coincidence: true,
            wy: 0,
            wx: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            dots: 0,
            window_line: 0,
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
            }
            if self.ly < FIRST_VBLANK_LINE {
                self.update_visible_mode();
            }
        }
        std::mem::take(&mut self.requested)
    }

    /// Reads VRAM, OAM or a register; other addresses are open bus.
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x8000..=0x9FFF => self.vram_byte(addr),
            0xFE00..=0xFE9F => self
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
            0x8000..=0x9FFF => {
                if let Some(cell) = self.vram.get_mut(usize::from(addr - VRAM_BASE)) {
                    *cell = val;
                }
            }
            0xFE00..=0xFE9F => self.write_oam(usize::from(addr - OAM_BASE), val),
            0xFF40 => self.write_lcdc(val),
            0xFF41 => {
                self.stat_sources = val & STAT_SOURCES;
                self.update_stat_line();
            }
            0xFF42 => self.scy = val,
            0xFF43 => self.scx = val,
            0xFF45 => {
                self.lyc = val;
                self.compare_lyc();
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
            self.ly = 0;
            self.dots = 0;
            self.window_line = 0;
            self.mode = Mode::HBlank;
            self.update_stat_line();
        }
        if !was_on && is_on {
            self.skip_oam_scan = true;
            self.compare_lyc();
        }
    }

    fn start_line(&mut self) {
        self.dots = 0;
        self.ly = if self.ly == LAST_LINE { 0 } else { self.ly + 1 };
        if self.ly == FIRST_VBLANK_LINE {
            self.window_line = 0;
            self.requested |= IRQ_VBLANK;
            self.mode = Mode::VBlank;
        }
        self.compare_lyc();
    }

    fn update_visible_mode(&mut self) {
        let mode = if self.dots <= OAM_SCAN_LAST_DOT {
            if self.skip_oam_scan {
                return;
            }
            Mode::OamScan
        } else if self.dots <= DRAWING_LAST_DOT {
            Mode::Drawing
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

    fn compare_lyc(&mut self) {
        if self.lcdc & LCDC_ON != 0 {
            self.coincidence = self.ly == self.lyc;
            self.update_stat_line();
        }
    }

    fn update_stat_line(&mut self) {
        let mode_source = if self.lcdc & LCDC_ON != 0 {
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
