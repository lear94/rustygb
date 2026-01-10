pub struct Ppu {
    pub buffer: Vec<u8>,
    vram: [u8; 8192],
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
            buffer: vec![0; 160 * 144 * 4],
            vram: [0; 8192],
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

    pub fn tick(&mut self, cycles: u8, if_reg: &mut u8) {
        if (self.lcdc & 0x80) == 0 {
            return;
        }

        self.dots += cycles as u32;

        if self.dots >= 456 {
            self.dots -= 456;
            self.ly += 1;

            if self.ly == self.lyc {
                self.stat |= 4;
                if (self.stat & 0x40) != 0 {
                    *if_reg |= 2;
                }
            } else {
                self.stat &= !4;
            }

            if self.ly == 144 {
                self.mode = 1;
                self.stat = (self.stat & 0xFC) | 1;
                *if_reg |= 1; // VBlank Interrupt
                if (self.stat & 0x10) != 0 {
                    *if_reg |= 2;
                }
                self.window_line = 0;
            }
        }

        if self.ly > 153 {
            self.ly = 0;
            self.dots = 0;
            self.mode = 2;
            self.stat = (self.stat & 0xFC) | 2;
            if (self.stat & 0x20) != 0 {
                *if_reg |= 2;
            }
        }

        if self.ly < 144 {
            if self.dots <= 80 {
                if self.mode != 2 {
                    self.mode = 2;
                    self.stat = (self.stat & 0xFC) | 2;
                    if (self.stat & 0x20) != 0 {
                        *if_reg |= 2;
                    }
                }
            } else if self.dots <= 252 {
                if self.mode != 3 {
                    self.mode = 3;
                    self.stat = (self.stat & 0xFC) | 3;
                }
            } else {
                if self.mode != 0 {
                    self.mode = 0;
                    self.stat = (self.stat & 0xFC) | 0;
                    if (self.stat & 0x08) != 0 {
                        *if_reg |= 2;
                    }
                    self.render();
                }
            }
        }
    }

    fn render(&mut self) {
        let bg_enable = (self.lcdc & 1) != 0;
        let win_enable = (self.lcdc & 0x20) != 0;
        let sprite_enable = (self.lcdc & 2) != 0;

        let bg_tile_map = if (self.lcdc & 8) != 0 { 0x9C00 } else { 0x9800 };
        let win_tile_map = if (self.lcdc & 0x40) != 0 {
            0x9C00
        } else {
            0x9800
        };

        let w_active = win_enable && self.wx <= 166 && self.ly >= self.wy;
        let mut render_win = false;
        if w_active {
            render_win = true;
        }

        let current_win_y = self.window_line;
        let mut bg_prio = [false; 160];

        for x in 0..160 {
            let using_win = render_win && (x + 7) >= self.wx;
            let tile_map = if using_win { win_tile_map } else { bg_tile_map };
            let y_pos = if using_win {
                current_win_y
            } else {
                self.ly.wrapping_add(self.scy)
            };
            let x_pos = if using_win {
                (x + 7).wrapping_sub(self.wx)
            } else {
                x.wrapping_add(self.scx)
            };

            let tile_row = (y_pos / 8) as u16;
            let tile_col = (x_pos / 8) as u16;
            let tid_addr = tile_map + tile_row * 32 + tile_col;
            let tid = self.vram[(tid_addr - 0x8000) as usize];

            let data_addr = if (self.lcdc & 0x10) != 0 {
                0x8000 + (tid as u16) * 16
            } else {
                let signed_tid = tid as i8 as i32;
                (0x9000 + signed_tid * 16) as u16
            };

            let line_in_tile = (y_pos % 8) as u16;
            let b1 = self.vram[(data_addr - 0x8000 + line_in_tile * 2) as usize];
            let b2 = self.vram[(data_addr - 0x8000 + line_in_tile * 2 + 1) as usize];

            let bit = 7 - (x_pos % 8);
            let color_idx = ((b1 >> bit) & 1) | (((b2 >> bit) & 1) << 1);
            bg_prio[x as usize] = bg_enable && (color_idx != 0);
            let final_color = if bg_enable {
                self.pal(color_idx, self.bgp)
            } else {
                self.pal(0, self.bgp)
            };
            self.plot(x, self.ly, final_color);
        }

        if render_win {
            self.window_line += 1;
        }
        if sprite_enable {
            self.sprites(&bg_prio);
        }
    }

    fn sprites(&mut self, bg_prio: &[bool; 160]) {
        let h = if (self.lcdc & 4) != 0 { 16 } else { 8 };
        for i in 0..40 {
            let idx = 39 - i;
            let sy = self.oam[idx * 4] as i16 - 16;
            let sx = self.oam[idx * 4 + 1] as i16 - 8;
            let t = self.oam[idx * 4 + 2];
            let f = self.oam[idx * 4 + 3];

            if (self.ly as i16) >= sy && (self.ly as i16) < sy + h {
                let line = self.ly as i16 - sy;
                let actual_line = if (f & 0x40) != 0 { h - 1 - line } else { line };
                let tile_idx = if h == 16 { t & 0xFE } else { t };
                let addr = 0x8000 + (tile_idx as u16 * 16) + (actual_line as u16 * 2);
                let b1 = self.vram[(addr - 0x8000) as usize];
                let b2 = self.vram[(addr - 0x8000 + 1) as usize];

                for px in 0..8 {
                    let pixel_x = sx + px;
                    if pixel_x >= 0 && pixel_x < 160 {
                        let bit = if (f & 0x20) != 0 { px } else { 7 - px };
                        let c = ((b1 >> bit) & 1) | (((b2 >> bit) & 1) << 1);
                        if c != 0 {
                            let prio_bg = (f & 0x80) != 0;
                            let bg_opaque = bg_prio[pixel_x as usize];
                            if !prio_bg || !bg_opaque {
                                let pal_reg = if (f & 0x10) != 0 {
                                    self.obp1
                                } else {
                                    self.obp0
                                };
                                self.plot(pixel_x as u8, self.ly, self.pal(c, pal_reg));
                            }
                        }
                    }
                }
            }
        }
    }

    fn pal(&self, i: u8, p: u8) -> [u8; 4] {
        match (p >> (i * 2)) & 3 {
            0 => [224, 248, 208, 255],
            1 => [136, 192, 112, 255],
            2 => [52, 104, 86, 255],
            3 => [8, 24, 32, 255],
            _ => [0, 0, 0, 255],
        }
    }

    fn plot(&mut self, x: u8, y: u8, c: [u8; 4]) {
        let i = (y as usize * 160 + x as usize) * 4;
        self.buffer[i..i + 4].copy_from_slice(&c);
    }

    pub fn read(&self, a: u16) -> u8 {
        match a {
            0x8000..=0x9FFF => self.vram[(a - 0x8000) as usize],
            0xFE00..=0xFE9F => self.oam[(a - 0xFE00) as usize],
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

    pub fn write(&mut self, a: u16, v: u8) {
        match a {
            0x8000..=0x9FFF => self.vram[(a - 0x8000) as usize] = v,
            0xFE00..=0xFE9F => self.oam[(a - 0xFE00) as usize] = v,
            0xFF40 => {
                let old_on = (self.lcdc & 0x80) != 0;
                self.lcdc = v;
                if old_on && (v & 0x80) == 0 {
                    self.ly = 0;
                    self.dots = 0;
                    self.mode = 0;
                    self.stat &= 0xFC;
                    self.window_line = 0;
                }
            }
            0xFF41 => self.stat = (self.stat & 0x07) | (v & 0x78),
            0xFF42 => self.scy = v,
            0xFF43 => self.scx = v,
            0xFF44 => {}
            0xFF45 => self.lyc = v,
            0xFF47 => self.bgp = v,
            0xFF48 => self.obp0 = v,
            0xFF49 => self.obp1 = v,
            0xFF4A => self.wy = v,
            0xFF4B => self.wx = v,
            _ => {}
        }
    }
    pub fn write_oam(&mut self, i: usize, v: u8) {
        self.oam[i] = v;
    }
}