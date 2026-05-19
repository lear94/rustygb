//! Pixel Processing Unit (PPU).
//!
//! Scanline-based renderer for the original DMG. The implementation favours
//! clarity over absolute hardware accuracy: each scanline is drawn in one
//! pass (background → window → sprites) at the start of mode 0 (H-Blank).
//! Sub-scanline timing tricks (FIFO, mid-line palette swaps, …) are not
//! modelled, but every other game-visible behaviour — STAT interrupts,
//! V-Blank cadence, sprite priority, window tracking — is.

/// Visible display width in pixels.
pub const SCREEN_WIDTH: usize = 160;
/// Visible display height in pixels.
pub const SCREEN_HEIGHT: usize = 144;
/// Total dot cycles per scanline (visible + H-Blank).
const DOTS_PER_LINE: u32 = 456;
/// First scanline of V-Blank.
const VBLANK_START_LINE: u8 = 144;
/// Last virtual scanline before wrapping back to 0.
const LAST_LINE: u8 = 153;

/// Pixel Processing Unit. The output frame is stored in `buffer` as
/// 160×144 RGBA bytes ready to be uploaded to a texture.
pub struct Ppu {
    /// Front buffer in RGBA8 format (`SCREEN_WIDTH * SCREEN_HEIGHT * 4`).
    pub buffer: Vec<u8>,

    vram: [u8; 8 * 1024],
    oam: [u8; 160],

    lcdc: u8,
    stat: u8,
    scy: u8,
    scx: u8,
    ly: u8,
    lyc: u8,
    wy: u8,
    wx: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,

    dots: u32,
    window_line: u8,
    mode: u8,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            buffer: vec![0; SCREEN_WIDTH * SCREEN_HEIGHT * 4],
            vram: [0; 8 * 1024],
            oam: [0; 160],
            lcdc: 0x91,
            stat: 0x85,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            wy: 0,
            wx: 0,
            bgp: 0xE4,
            obp0: 0xE4,
            obp1: 0xE4,
            dots: 0,
            window_line: 0,
            mode: 0,
        }
    }

    /// Advance the PPU by `cycles` T-cycles. Sets the appropriate bits in
    /// `if_reg` for the V-Blank and STAT interrupts.
    pub fn tick(&mut self, cycles: u8, if_reg: &mut u8) {
        if (self.lcdc & 0x80) == 0 {
            return;
        }

        self.dots += cycles as u32;

        if self.dots >= DOTS_PER_LINE {
            self.dots -= DOTS_PER_LINE;
            self.ly += 1;

            if self.ly == self.lyc {
                self.stat |= 0x04;
                if (self.stat & 0x40) != 0 {
                    *if_reg |= 0x02;
                }
            } else {
                self.stat &= !0x04;
            }

            if self.ly == VBLANK_START_LINE {
                self.mode = 1;
                self.stat = (self.stat & 0xFC) | 1;
                *if_reg |= 0x01;
                if (self.stat & 0x10) != 0 {
                    *if_reg |= 0x02;
                }
                self.window_line = 0;
            }
        }

        if self.ly > LAST_LINE {
            self.ly = 0;
            self.dots = 0;
            self.mode = 2;
            self.stat = (self.stat & 0xFC) | 2;
            if (self.stat & 0x20) != 0 {
                *if_reg |= 0x02;
            }
        }

        if self.ly < VBLANK_START_LINE {
            self.update_visible_mode(if_reg);
        }
    }

    fn update_visible_mode(&mut self, if_reg: &mut u8) {
        if self.dots <= 80 {
            if self.mode != 2 {
                self.mode = 2;
                self.stat = (self.stat & 0xFC) | 2;
                if (self.stat & 0x20) != 0 {
                    *if_reg |= 0x02;
                }
            }
        } else if self.dots <= 252 {
            if self.mode != 3 {
                self.mode = 3;
                self.stat = (self.stat & 0xFC) | 3;
            }
        } else if self.mode != 0 {
            self.mode = 0;
            self.stat &= 0xFC;
            if (self.stat & 0x08) != 0 {
                *if_reg |= 0x02;
            }
            self.render_scanline();
        }
    }

    /// Render the current scanline to `buffer`. Background and window are
    /// drawn first, then sprites are overlaid using the priority bit.
    fn render_scanline(&mut self) {
        let bg_enable = (self.lcdc & 0x01) != 0;
        let win_enable = (self.lcdc & 0x20) != 0;
        let sprite_enable = (self.lcdc & 0x02) != 0;

        let bg_tile_map: u16 = if (self.lcdc & 0x08) != 0 {
            0x9C00
        } else {
            0x9800
        };
        let win_tile_map: u16 = if (self.lcdc & 0x40) != 0 {
            0x9C00
        } else {
            0x9800
        };

        let window_active = win_enable && self.wx <= 166 && self.ly >= self.wy;
        let current_win_y = self.window_line;
        let mut bg_opaque = [false; SCREEN_WIDTH];

        for x in 0..SCREEN_WIDTH as u8 {
            let using_window = window_active && (x + 7) >= self.wx;
            let tile_map = if using_window {
                win_tile_map
            } else {
                bg_tile_map
            };
            let y_pos = if using_window {
                current_win_y
            } else {
                self.ly.wrapping_add(self.scy)
            };
            let x_pos = if using_window {
                (x + 7).wrapping_sub(self.wx)
            } else {
                x.wrapping_add(self.scx)
            };

            let tile_row = (y_pos / 8) as u16;
            let tile_col = (x_pos / 8) as u16;
            let tid_addr = tile_map + tile_row * 32 + tile_col;
            let tid = self.vram[(tid_addr - 0x8000) as usize];

            let data_addr = if (self.lcdc & 0x10) != 0 {
                0x8000u16 + (tid as u16) * 16
            } else {
                (0x9000i32 + (tid as i8 as i32) * 16) as u16
            };

            let line_in_tile = (y_pos % 8) as u16;
            let b1 = self.vram[(data_addr - 0x8000 + line_in_tile * 2) as usize];
            let b2 = self.vram[(data_addr - 0x8000 + line_in_tile * 2 + 1) as usize];

            let bit = 7 - (x_pos % 8);
            let color_idx = ((b1 >> bit) & 1) | (((b2 >> bit) & 1) << 1);
            bg_opaque[x as usize] = bg_enable && color_idx != 0;

            let palette_idx = if bg_enable { color_idx } else { 0 };
            self.plot(x, self.ly, self.pal(palette_idx, self.bgp));
        }

        if window_active {
            self.window_line += 1;
        }
        if sprite_enable {
            self.render_sprites(&bg_opaque);
        }
    }

    /// Sprite (OBJ) pass. Iterates OAM in reverse so lower indices win on
    /// X-collision, matching DMG hardware. Honours the BG-priority flag,
    /// 8×16 mode, X/Y mirroring and per-sprite palette selection.
    fn render_sprites(&mut self, bg_opaque: &[bool; SCREEN_WIDTH]) {
        let height: i16 = if (self.lcdc & 0x04) != 0 { 16 } else { 8 };

        for i in 0..40 {
            let idx = 39 - i;
            let sy = self.oam[idx * 4] as i16 - 16;
            let sx = self.oam[idx * 4 + 1] as i16 - 8;
            let tile = self.oam[idx * 4 + 2];
            let flags = self.oam[idx * 4 + 3];

            let ly = self.ly as i16;
            if ly < sy || ly >= sy + height {
                continue;
            }

            let line = ly - sy;
            let row = if (flags & 0x40) != 0 {
                height - 1 - line
            } else {
                line
            };
            let tile_idx = if height == 16 { tile & 0xFE } else { tile };
            let addr = (tile_idx as u16) * 16 + (row as u16) * 2;

            let b1 = self.vram[addr as usize];
            let b2 = self.vram[(addr + 1) as usize];
            let palette = if (flags & 0x10) != 0 {
                self.obp1
            } else {
                self.obp0
            };
            let behind_bg = (flags & 0x80) != 0;
            let x_flip = (flags & 0x20) != 0;

            for px in 0..8i16 {
                let pixel_x = sx + px;
                if pixel_x < 0 || pixel_x >= SCREEN_WIDTH as i16 {
                    continue;
                }
                let bit = if x_flip { px as u8 } else { 7 - px as u8 };
                let color_idx = ((b1 >> bit) & 1) | (((b2 >> bit) & 1) << 1);
                if color_idx == 0 {
                    continue;
                }
                if behind_bg && bg_opaque[pixel_x as usize] {
                    continue;
                }
                self.plot(pixel_x as u8, self.ly, self.pal(color_idx, palette));
            }
        }
    }

    /// Translate a 2-bit color index through a Game Boy palette register
    /// to one of the four canonical "DMG green" RGBA colours.
    fn pal(&self, color_idx: u8, palette: u8) -> [u8; 4] {
        match (palette >> (color_idx * 2)) & 3 {
            0 => [224, 248, 208, 255],
            1 => [136, 192, 112, 255],
            2 => [52, 104, 86, 255],
            _ => [8, 24, 32, 255],
        }
    }

    fn plot(&mut self, x: u8, y: u8, color: [u8; 4]) {
        let offset = (y as usize * SCREEN_WIDTH + x as usize) * 4;
        self.buffer[offset..offset + 4].copy_from_slice(&color);
    }

    /// Memory-mapped read of VRAM/OAM/registers.
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize],
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize],
            0xFF40 => self.lcdc,
            0xFF41 => self.stat | 0x80,
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            _ => 0xFF,
        }
    }

    /// Memory-mapped write of VRAM/OAM/registers. Turning the LCD off
    /// resets the scanline counter and clears the active mode.
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize] = val,
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize] = val,
            0xFF40 => {
                let was_on = (self.lcdc & 0x80) != 0;
                self.lcdc = val;
                if was_on && (val & 0x80) == 0 {
                    self.ly = 0;
                    self.dots = 0;
                    self.mode = 0;
                    self.stat &= 0xFC;
                    self.window_line = 0;
                }
            }
            // Bits 0-2 of STAT are read-only.
            0xFF41 => self.stat = (self.stat & 0x07) | (val & 0x78),
            0xFF42 => self.scy = val,
            0xFF43 => self.scx = val,
            0xFF44 => {}
            0xFF45 => self.lyc = val,
            0xFF47 => self.bgp = val,
            0xFF48 => self.obp0 = val,
            0xFF49 => self.obp1 = val,
            0xFF4A => self.wy = val,
            0xFF4B => self.wx = val,
            _ => {}
        }
    }

    /// Direct OAM write used by the DMA controller in the bus.
    pub fn write_oam(&mut self, index: usize, val: u8) {
        self.oam[index] = val;
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}
