//! CPU side of the PPU: video memory and registers.

use super::{
    Mode, Ppu, FIRST_VBLANK_LINE, OAM_BASE, OAM_SCAN_LAST_DOT, STAT_COINCIDENCE, STAT_SOURCES,
    STAT_UNUSED, VRAM_BASE,
};
use crate::hardware::OPEN_BUS;
use crate::memory_map::{
    BGP, DMA, LCDC, LY, LYC, OAM_END, OAM_START, OBP0, OBP1, SCX, SCY, STAT, VRAM_END, VRAM_START,
    WX, WY,
};

impl Ppu {
    /// Reads VRAM, OAM or a register; other addresses are open bus.
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            VRAM_START..=VRAM_END if self.vram_readable() => self.vram_byte(addr),
            OAM_START..=OAM_END if self.oam_readable() => self
                .oam
                .get(usize::from(addr - OAM_BASE))
                .copied()
                .unwrap_or(OPEN_BUS),
            LCDC..=DMA => self.read_control(addr),
            BGP..=WX => self.read_palette_or_window(addr),
            _ => OPEN_BUS,
        }
    }

    /// Writes VRAM, OAM or a register; other addresses are ignored.
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            VRAM_START..=VRAM_END if self.vram_writable() => {
                if let Some(cell) = self.vram.get_mut(usize::from(addr - VRAM_BASE)) {
                    *cell = val;
                }
            }
            OAM_START..=OAM_END if self.oam_writable() => {
                self.write_oam(usize::from(addr - OAM_BASE), val);
            }
            LCDC..=DMA => self.write_control(addr, val),
            BGP..=WX => self.write_palette_or_window(addr, val),
            _ => {}
        }
    }

    fn read_control(&self, addr: u16) -> u8 {
        match addr {
            LCDC => self.lcdc,
            STAT => self.stat(),
            SCY => self.scy,
            SCX => self.scx,
            LY => self.ly,
            LYC => self.lyc,
            _ => OPEN_BUS,
        }
    }

    fn write_control(&mut self, addr: u16, val: u8) {
        match addr {
            LCDC => self.write_lcdc(val),
            STAT => self.write_stat(val),
            SCY => self.scy = val,
            SCX => self.scx = val,
            LYC => {
                self.lyc = val;
                self.compare_lyc(self.compared_line);
            }
            _ => {}
        }
    }

    fn read_palette_or_window(&self, addr: u16) -> u8 {
        match addr {
            BGP => self.bgp,
            OBP0 => self.obp0,
            OBP1 => self.obp1,
            WY => self.wy,
            _ => self.wx,
        }
    }

    fn write_palette_or_window(&mut self, addr: u16, val: u8) {
        match addr {
            BGP => self.bgp = val,
            OBP0 => self.obp0 = val,
            OBP1 => self.obp1 = val,
            WY => self.wy = val,
            _ => self.wx = val,
        }
    }

    fn write_stat(&mut self, val: u8) {
        // DMG quirk: for an instant every source is enabled.
        self.stat_sources = STAT_SOURCES;
        self.update_stat_line();
        self.stat_sources = val & STAT_SOURCES;
        self.update_stat_line();
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

    pub(super) fn vram_byte(&self, addr: u16) -> u8 {
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
}
