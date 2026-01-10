use crate::bus::Motherboard;

const Z: u8 = 0x80;
const N: u8 = 0x40;
const H: u8 = 0x20;
const C: u8 = 0x10;

pub struct Cpu {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub ime: bool,
    pub halted: bool,
    pub ei_pending: bool,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            a: 0,
            f: 0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            sp: 0,
            pc: 0,
            ime: false,
            halted: false,
            ei_pending: false,
        }
    }

    pub fn reset_to_boot(&mut self) {
        self.pc = 0x100;
        self.a = 0x01;
        self.f = 0xB0;
        self.b = 0x00;
        self.c = 0x13;
        self.d = 0x00;
        self.e = 0xD8;
        self.h = 0x01;
        self.l = 0x4D;
        self.sp = 0xFFFE;
    }

    pub fn step(&mut self, bus: &mut Motherboard) -> u8 {
        let ie = bus.read_byte(0xFFFF);
        let if_reg = bus.read_byte(0xFF0F);
        let pending = ie & if_reg & 0x1F;

        if self.halted {
            if pending != 0 {
                self.halted = false;
            } else {
                return 4;
            }
        }

        if self.ime && pending != 0 {
            self.ime = false;
            let bit = pending.trailing_zeros();
            bus.write_byte(0xFF0F, if_reg & !(1 << bit));
            self.push_word(bus, self.pc);
            self.pc = 0x40 + (bit as u16 * 8);
            return 20;
        }

        if self.ei_pending {
            self.ime = true;
            self.ei_pending = false;
        }

        let op = bus.read_byte(self.pc);
        self.pc = self.pc.wrapping_add(1);
        self.execute(op, bus)
    }

    fn execute(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
        match op {
            0x00 => 4,
            0x06 => {
                let v = self.next_byte(bus);
                self.b = v;
                8
            }
            0x0E => {
                let v = self.next_byte(bus);
                self.c = v;
                8
            }
            0x16 => {
                let v = self.next_byte(bus);
                self.d = v;
                8
            }
            0x1E => {
                let v = self.next_byte(bus);
                self.e = v;
                8
            }
            0x26 => {
                let v = self.next_byte(bus);
                self.h = v;
                8
            }
            0x2E => {
                let v = self.next_byte(bus);
                self.l = v;
                8
            }
            0x3E => {
                let v = self.next_byte(bus);
                self.a = v;
                8
            }
            0x36 => {
                let v = self.next_byte(bus);
                bus.write_byte(self.get_hl(), v);
                12
            }
            0x40..=0x75 | 0x77..=0x7F => self.exec_ld_r_r(op, bus),
            0x01 => {
                let c = self.next_byte(bus);
                let b = self.next_byte(bus);
                self.c = c;
                self.b = b;
                12
            }
            0x11 => {
                let e = self.next_byte(bus);
                let d = self.next_byte(bus);
                self.e = e;
                self.d = d;
                12
            }
            0x21 => {
                let l = self.next_byte(bus);
                let h = self.next_byte(bus);
                self.l = l;
                self.h = h;
                12
            }
            0x31 => {
                let v = self.next_word(bus);
                self.sp = v;
                12
            }
            0x0A => {
                self.a = bus.read_byte(self.get_bc());
                8
            }
            0x1A => {
                self.a = bus.read_byte(self.get_de());
                8
            }
            0xFA => {
                let a = self.next_word(bus);
                self.a = bus.read_byte(a);
                16
            }
            0x02 => {
                bus.write_byte(self.get_bc(), self.a);
                8
            }
            0x12 => {
                bus.write_byte(self.get_de(), self.a);
                8
            }
            0xEA => {
                let a = self.next_word(bus);
                bus.write_byte(a, self.a);
                16
            }
            0x22 => {
                bus.write_byte(self.get_hl(), self.a);
                self.set_hl(self.get_hl().wrapping_add(1));
                8
            }
            0x2A => {
                self.a = bus.read_byte(self.get_hl());
                self.set_hl(self.get_hl().wrapping_add(1));
                8
            }
            0x32 => {
                bus.write_byte(self.get_hl(), self.a);
                self.set_hl(self.get_hl().wrapping_sub(1));
                8
            }
            0x3A => {
                self.a = bus.read_byte(self.get_hl());
                self.set_hl(self.get_hl().wrapping_sub(1));
                8
            }
            0xC5 => {
                self.push_word(bus, self.get_bc());
                16
            }
            0xD5 => {
                self.push_word(bus, self.get_de());
                16
            }
            0xE5 => {
                self.push_word(bus, self.get_hl());
                16
            }
            0xF5 => {
                self.push_word(bus, (self.a as u16) << 8 | self.f as u16);
                16
            }
            0xC1 => {
                let v = self.pop_word(bus);
                self.b = (v >> 8) as u8;
                self.c = v as u8;
                12
            }
            0xD1 => {
                let v = self.pop_word(bus);
                self.d = (v >> 8) as u8;
                self.e = v as u8;
                12
            }
            0xE1 => {
                let v = self.pop_word(bus);
                self.h = (v >> 8) as u8;
                self.l = v as u8;
                12
            }
            0xF1 => {
                let v = self.pop_word(bus);
                self.a = (v >> 8) as u8;
                self.f = (v as u8) & 0xF0;
                12
            }
            0xC3 => {
                self.pc = self.next_word(bus);
                16
            }
            0xE9 => {
                self.pc = self.get_hl();
                4
            }
            0x18 => {
                let o = self.next_byte(bus) as i8;
                self.pc = self.pc.wrapping_add(o as u16);
                12
            }
            0x20 => self.jr_cond(bus, !self.z()),
            0x28 => self.jr_cond(bus, self.z()),
            0x30 => self.jr_cond(bus, !self.c_flg()),
            0x38 => self.jr_cond(bus, self.c_flg()),
            0xC2 => self.jp_cond(bus, !self.z()),
            0xCA => self.jp_cond(bus, self.z()),
            0xD2 => self.jp_cond(bus, !self.c_flg()),
            0xDA => self.jp_cond(bus, self.c_flg()),
            0xCD => {
                let d = self.next_word(bus);
                self.push_word(bus, self.pc);
                self.pc = d;
                24
            }
            0xC4 => self.call_cond(bus, !self.z()),
            0xCC => self.call_cond(bus, self.z()),
            0xD4 => self.call_cond(bus, !self.c_flg()),
            0xDC => self.call_cond(bus, self.c_flg()),
            0xC9 => {
                self.pc = self.pop_word(bus);
                16
            }
            0xD9 => {
                self.pc = self.pop_word(bus);
                self.ime = true;
                16
            }
            0xC0 => self.ret_cond(bus, !self.z()),
            0xC8 => self.ret_cond(bus, self.z()),
            0xD0 => self.ret_cond(bus, !self.c_flg()),
            0xD8 => self.ret_cond(bus, self.c_flg()),
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                self.push_word(bus, self.pc);
                self.pc = (op - 0xC7) as u16;
                16
            }
            0x07 => {
                let c = self.a >> 7;
                self.a = (self.a << 1) | c;
                self.f = if c != 0 { C } else { 0 };
                4
            }
            0x0F => {
                let c = self.a & 1;
                self.a = (self.a >> 1) | (c << 7);
                self.f = if c != 0 { C } else { 0 };
                4
            }
            0x17 => {
                let c = self.a >> 7;
                let old_c = if (self.f & C) != 0 { 1 } else { 0 };
                self.a = (self.a << 1) | old_c;
                self.f = if c != 0 { C } else { 0 };
                4
            }
            0x1F => {
                let c = self.a & 1;
                let old_c = if (self.f & C) != 0 { 0x80 } else { 0 };
                self.a = (self.a >> 1) | old_c;
                self.f = if c != 0 { C } else { 0 };
                4
            }
            0x80..=0x87 => {
                let v = self.get_r(op & 7, bus);
                self.add(v);
                4
            }
            0x88..=0x8F => {
                let v = self.get_r(op & 7, bus);
                self.adc(v);
                4
            }
            0x90..=0x97 => {
                let v = self.get_r(op & 7, bus);
                self.sub(v);
                4
            }
            0x98..=0x9F => {
                let v = self.get_r(op & 7, bus);
                self.sbc(v);
                4
            }
            0xA0..=0xA7 => {
                let v = self.get_r(op & 7, bus);
                self.and(v);
                4
            }
            0xA8..=0xAF => {
                let v = self.get_r(op & 7, bus);
                self.xor(v);
                4
            }
            0xB0..=0xB7 => {
                let v = self.get_r(op & 7, bus);
                self.or(v);
                4
            }
            0xB8..=0xBF => {
                let v = self.get_r(op & 7, bus);
                self.cp(v);
                4
            }
            0xC6 => {
                let v = self.next_byte(bus);
                self.add(v);
                8
            }
            0xCE => {
                let v = self.next_byte(bus);
                self.adc(v);
                8
            }
            0xD6 => {
                let v = self.next_byte(bus);
                self.sub(v);
                8
            }
            0xDE => {
                let v = self.next_byte(bus);
                self.sbc(v);
                8
            }
            0xE6 => {
                let v = self.next_byte(bus);
                self.and(v);
                8
            }
            0xEE => {
                let v = self.next_byte(bus);
                self.xor(v);
                8
            }
            0xF6 => {
                let v = self.next_byte(bus);
                self.or(v);
                8
            }
            0xFE => {
                let v = self.next_byte(bus);
                self.cp(v);
                8
            }
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x3C | 0x34 => self.exec_inc_dec(op, bus),
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x3D | 0x35 => self.exec_inc_dec(op, bus),
            0x09 => {
                let v = self.get_bc();
                self.add_hl(v);
                8
            }
            0x19 => {
                let v = self.get_de();
                self.add_hl(v);
                8
            }
            0x29 => {
                let v = self.get_hl();
                self.add_hl(v);
                8
            }
            0x39 => {
                let v = self.sp;
                self.add_hl(v);
                8
            }
            0x03 => {
                let v = self.get_bc().wrapping_add(1);
                self.set_bc(v);
                8
            }
            0x13 => {
                let v = self.get_de().wrapping_add(1);
                self.set_de(v);
                8
            }
            0x23 => {
                let v = self.get_hl().wrapping_add(1);
                self.set_hl(v);
                8
            }
            0x33 => {
                self.sp = self.sp.wrapping_add(1);
                8
            }
            0x0B => {
                let v = self.get_bc().wrapping_sub(1);
                self.set_bc(v);
                8
            }
            0x1B => {
                let v = self.get_de().wrapping_sub(1);
                self.set_de(v);
                8
            }
            0x2B => {
                let v = self.get_hl().wrapping_sub(1);
                self.set_hl(v);
                8
            }
            0x3B => {
                self.sp = self.sp.wrapping_sub(1);
                8
            }
            0x27 => {
                self.daa();
                4
            }
            0x2F => {
                self.a = !self.a;
                self.set_n(true);
                self.set_h(true);
                4
            }
            0x3F => {
                let c = !self.c_flg();
                self.set_c(c);
                self.set_n(false);
                self.set_h(false);
                4
            }
            0x37 => {
                self.set_c(true);
                self.set_n(false);
                self.set_h(false);
                4
            }
            0xF8 => {
                let r = self.next_byte(bus) as i8 as u16;
                let res = self.sp.wrapping_add(r);
                self.f = 0;
                if (self.sp & 0xF) + (r & 0xF) > 0xF {
                    self.set_h(true);
                }
                if (self.sp & 0xFF) + (r & 0xFF) > 0xFF {
                    self.set_c(true);
                }
                self.set_hl(res);
                12
            }
            0xE8 => {
                let r = self.next_byte(bus) as i8 as u16;
                let res = self.sp.wrapping_add(r);
                self.f = 0;
                if (self.sp & 0xF) + (r & 0xF) > 0xF {
                    self.set_h(true);
                }
                if (self.sp & 0xFF) + (r & 0xFF) > 0xFF {
                    self.set_c(true);
                }
                self.sp = res;
                16
            }
            0xF9 => {
                self.sp = self.get_hl();
                8
            }
            0xE0 => {
                let v = self.next_byte(bus);
                bus.write_byte(0xFF00 + v as u16, self.a);
                12
            }
            0xF0 => {
                let v = self.next_byte(bus);
                self.a = bus.read_byte(0xFF00 + v as u16);
                12
            }
            0xE2 => {
                bus.write_byte(0xFF00 + self.c as u16, self.a);
                8
            }
            0xF2 => {
                self.a = bus.read_byte(0xFF00 + self.c as u16);
                8
            }
            0x10 => {
                let _ = self.next_byte(bus);
                4
            }
            0x76 => {
                self.halted = true;
                4
            }
            0xF3 => {
                self.ime = false;
                4
            }
            0xFB => {
                self.ei_pending = true;
                4
            }
            0xCB => {
                let op = self.next_byte(bus);
                self.execute_cb(op, bus)
            }
            _ => 4,
        }
    }

    fn execute_cb(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
        let r_idx = op & 0x07;
        let mut v = self.get_r(r_idx, bus);

        match op >> 6 {
            0 => {
                match (op >> 3) & 0x07 {
                    0 => {
                        let c = v >> 7;
                        v = (v << 1) | c;
                        self.f = if v == 0 { Z } else { 0 };
                        if c != 0 {
                            self.f |= C;
                        }
                    }
                    1 => {
                        let c = v & 1;
                        v = (v >> 1) | (c << 7);
                        self.f = if v == 0 { Z } else { 0 };
                        if c != 0 {
                            self.f |= C;
                        }
                    }
                    2 => {
                        let c = v >> 7;
                        v = (v << 1) | if self.c_flg() { 1 } else { 0 };
                        self.f = if v == 0 { Z } else { 0 };
                        if c != 0 {
                            self.f |= C;
                        }
                    }
                    3 => {
                        let c = v & 1;
                        v = (v >> 1) | if self.c_flg() { 0x80 } else { 0 };
                        self.f = if v == 0 { Z } else { 0 };
                        if c != 0 {
                            self.f |= C;
                        }
                    }
                    4 => {
                        let c = v >> 7;
                        v <<= 1;
                        self.f = if v == 0 { Z } else { 0 };
                        if c != 0 {
                            self.f |= C;
                        }
                    }
                    5 => {
                        let c = v & 1;
                        v = (v as i8 >> 1) as u8;
                        self.f = if v == 0 { Z } else { 0 };
                        if c != 0 {
                            self.f |= C;
                        }
                    }
                    6 => {
                        v = (v << 4) | (v >> 4);
                        self.f = if v == 0 { Z } else { 0 };
                    }
                    _ => {
                        let c = v & 1;
                        v >>= 1;
                        self.f = if v == 0 { Z } else { 0 };
                        if c != 0 {
                            self.f |= C;
                        }
                    }
                }
                self.set_r(r_idx, v, bus);
                if r_idx == 6 {
                    16
                } else {
                    8
                }
            }
            1 => {
                let bit = (op >> 3) & 0x07;
                let res = v & (1 << bit);
                self.f = (self.f & C) | H | if res == 0 { Z } else { 0 };
                if r_idx == 6 {
                    12
                } else {
                    8
                }
            }
            2 => {
                let bit = (op >> 3) & 0x07;
                v &= !(1 << bit);
                self.set_r(r_idx, v, bus);
                if r_idx == 6 {
                    16
                } else {
                    8
                }
            }
            _ => {
                let bit = (op >> 3) & 0x07;
                v |= 1 << bit;
                self.set_r(r_idx, v, bus);
                if r_idx == 6 {
                    16
                } else {
                    8
                }
            }
        }
    }

    fn exec_ld_r_r(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
        if op == 0x76 {
            return 4;
        }
        let s = self.get_r(op & 7, bus);
        self.set_r((op >> 3) & 7, s, bus);
        if (op & 7) == 6 || ((op >> 3) & 7) == 6 {
            8
        } else {
            4
        }
    }

    fn exec_inc_dec(&mut self, op: u8, bus: &mut Motherboard) -> u8 {
        let i = (op >> 3) & 7;
        let mut v = self.get_r(i, bus);
        if (op & 1) == 0 {
            let r = v.wrapping_add(1);
            self.f =
                (self.f & C) | if r == 0 { Z } else { 0 } | if (v & 0xF) == 0xF { H } else { 0 };
            v = r;
        } else {
            let r = v.wrapping_sub(1);
            self.f =
                (self.f & C) | N | if r == 0 { Z } else { 0 } | if (v & 0xF) == 0 { H } else { 0 };
            v = r;
        }
        self.set_r(i, v, bus);
        if i == 6 {
            12
        } else {
            4
        }
    }

    fn add(&mut self, v: u8) {
        let r = self.a.wrapping_add(v);
        self.f = if r == 0 { Z } else { 0 }
            | if (self.a & 0xF) + (v & 0xF) > 0xF {
                H
            } else {
                0
            }
            | if (self.a as u16) + (v as u16) > 0xFF {
                C
            } else {
                0
            };
        self.a = r;
    }
    fn adc(&mut self, v: u8) {
        let c = if self.c_flg() { 1 } else { 0 };
        let r = self.a.wrapping_add(v).wrapping_add(c);
        self.f = if r == 0 { Z } else { 0 }
            | if (self.a & 0xF) + (v & 0xF) + c > 0xF {
                H
            } else {
                0
            }
            | if (self.a as u16) + (v as u16) + (c as u16) > 0xFF {
                C
            } else {
                0
            };
        self.a = r;
    }
    fn sub(&mut self, v: u8) {
        let r = self.a.wrapping_sub(v);
        self.f = N
            | if r == 0 { Z } else { 0 }
            | if (self.a & 0xF) < (v & 0xF) { H } else { 0 }
            | if self.a < v { C } else { 0 };
        self.a = r;
    }
    fn sbc(&mut self, v: u8) {
        let c = if self.c_flg() { 1 } else { 0 };
        let r = self.a.wrapping_sub(v).wrapping_sub(c);
        self.f = N
            | if r == 0 { Z } else { 0 }
            | if (self.a as u16) < (v as u16) + (c as u16) {
                C
            } else {
                0
            }
            | if (self.a & 0xF) < (v & 0xF) + c { H } else { 0 };
        self.a = r;
    }
    fn and(&mut self, v: u8) {
        self.a &= v;
        self.f = H | if self.a == 0 { Z } else { 0 };
    }
    fn or(&mut self, v: u8) {
        self.a |= v;
        self.f = if self.a == 0 { Z } else { 0 };
    }
    fn xor(&mut self, v: u8) {
        self.a ^= v;
        self.f = if self.a == 0 { Z } else { 0 };
    }
    fn cp(&mut self, v: u8) {
        self.f = N
            | if self.a == v { Z } else { 0 }
            | if (self.a & 0xF) < (v & 0xF) { H } else { 0 }
            | if self.a < v { C } else { 0 };
    }
    fn add_hl(&mut self, v: u16) {
        let hl = self.get_hl();
        let r = hl.wrapping_add(v);
        self.f = (self.f & Z)
            | if (hl & 0xFFF) + (v & 0xFFF) > 0xFFF {
                H
            } else {
                0
            }
            | if (hl as u32) + (v as u32) > 0xFFFF {
                C
            } else {
                0
            };
        self.set_hl(r);
    }
    fn daa(&mut self) {
        let mut a = self.a;
        let mut adj = 0;
        if self.h_flg() || (!self.n_flg() && (a & 0xF) > 9) {
            adj |= 0x06;
        }
        if self.c_flg() || (!self.n_flg() && a > 0x99) {
            adj |= 0x60;
            self.set_c(true);
        }
        a = if self.n_flg() {
            a.wrapping_sub(adj)
        } else {
            a.wrapping_add(adj)
        };
        self.f = (self.f & C) | if self.n_flg() { N } else { 0 } | if a == 0 { Z } else { 0 };
        self.a = a;
    }

    fn z(&self) -> bool {
        self.f & Z != 0
    }
    fn n_flg(&self) -> bool {
        self.f & N != 0
    }
    fn h_flg(&self) -> bool {
        self.f & H != 0
    }
    fn c_flg(&self) -> bool {
        self.f & C != 0
    }
    fn set_n(&mut self, v: bool) {
        if v {
            self.f |= N
        } else {
            self.f &= !N
        }
    }
    fn set_h(&mut self, v: bool) {
        if v {
            self.f |= H
        } else {
            self.f &= !H
        }
    }
    fn set_c(&mut self, v: bool) {
        if v {
            self.f |= C
        } else {
            self.f &= !C
        }
    }
    fn get_bc(&self) -> u16 {
        (self.b as u16) << 8 | self.c as u16
    }
    fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }
    fn get_de(&self) -> u16 {
        (self.d as u16) << 8 | self.e as u16
    }
    fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }
    fn get_hl(&self) -> u16 {
        (self.h as u16) << 8 | self.l as u16
    }
    fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }
    fn next_byte(&mut self, bus: &mut Motherboard) -> u8 {
        let v = bus.read_byte(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }
    fn next_word(&mut self, bus: &mut Motherboard) -> u16 {
        let l = self.next_byte(bus);
        let h = self.next_byte(bus);
        (h as u16) << 8 | l as u16
    }
    fn push_word(&mut self, bus: &mut Motherboard, v: u16) {
        self.sp = self.sp.wrapping_sub(2);
        bus.write_word(self.sp, v);
    }
    fn pop_word(&mut self, bus: &mut Motherboard) -> u16 {
        let v = bus.read_word(self.sp);
        self.sp = self.sp.wrapping_add(2);
        v
    }
    fn get_r(&self, i: u8, bus: &Motherboard) -> u8 {
        match i {
            0 => self.b,
            1 => self.c,
            2 => self.d,
            3 => self.e,
            4 => self.h,
            5 => self.l,
            6 => bus.read_byte(self.get_hl()),
            7 => self.a,
            _ => 0,
        }
    }
    fn set_r(&mut self, i: u8, v: u8, bus: &mut Motherboard) {
        match i {
            0 => self.b = v,
            1 => self.c = v,
            2 => self.d = v,
            3 => self.e = v,
            4 => self.h = v,
            5 => self.l = v,
            6 => bus.write_byte(self.get_hl(), v),
            7 => self.a = v,
            _ => {}
        }
    }
    fn jr_cond(&mut self, bus: &mut Motherboard, c: bool) -> u8 {
        let o = self.next_byte(bus) as i8;
        if c {
            self.pc = self.pc.wrapping_add(o as u16);
            12
        } else {
            8
        }
    }
    fn jp_cond(&mut self, bus: &mut Motherboard, c: bool) -> u8 {
        let d = self.next_word(bus);
        if c {
            self.pc = d;
            16
        } else {
            12
        }
    }
    fn call_cond(&mut self, bus: &mut Motherboard, c: bool) -> u8 {
        let d = self.next_word(bus);
        if c {
            self.push_word(bus, self.pc);
            self.pc = d;
            24
        } else {
            12
        }
    }
    fn ret_cond(&mut self, bus: &mut Motherboard, c: bool) -> u8 {
        if c {
            self.pc = self.pop_word(bus);
            20
        } else {
            8
        }
    }
}
