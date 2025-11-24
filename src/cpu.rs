use crate::bus::Bus;

pub const FLAG_CARRY: u8 = 0b0000_0001;
pub const FLAG_ZERO: u8 = 0b0000_0010;
pub const FLAG_INTERRUPT_DISABLE: u8 = 0b0000_0100;
pub const FLAG_DECIMAL: u8 = 0b0000_1000;
pub const FLAG_BREAK: u8 = 0b0001_0000;
pub const FLAG_T: u8 = 0b0010_0000;
pub const FLAG_OVERFLOW: u8 = 0b0100_0000;
pub const FLAG_NEGATIVE: u8 = 0b1000_0000;

/// HuC6280 CPU core.
/// Implements a growing subset of the instruction matrix shared with the 65C02,
/// covering common loads/stores, arithmetic, branches, and subroutine control.
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub status: u8,
    pub halted: bool,
    pub clock_high_speed: bool,
    waiting: bool,
    irq_pending: bool,
    nmi_pending: bool,
    last_opcode: u8,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0,
            status: FLAG_INTERRUPT_DISABLE | FLAG_T,
            halted: false,
            clock_high_speed: false,
            waiting: false,
            irq_pending: false,
            nmi_pending: false,
            last_opcode: 0,
        }
    }

    pub fn reset(&mut self, bus: &mut Bus) {
        self.sp = 0xFD;
        self.pc = bus.read_u16(0xFFFC);
        self.status = FLAG_INTERRUPT_DISABLE | FLAG_T;
        self.halted = false;
        self.clock_high_speed = false;
        self.waiting = false;
        self.irq_pending = false;
        self.nmi_pending = false;
        self.last_opcode = 0;
    }

    pub fn request_irq(&mut self) {
        self.irq_pending = true;
    }

    pub fn request_nmi(&mut self) {
        self.nmi_pending = true;
    }

    pub fn step(&mut self, bus: &mut Bus) -> u8 {
        if self.halted {
            return 0;
        }

        if !self.nmi_pending && bus.irq_pending() {
            self.irq_pending = true;
        }

        if self.nmi_pending {
            self.nmi_pending = false;
            return self.handle_interrupt(bus, 0xFFFA, false);
        }

        if (self.irq_pending || bus.irq_pending())
            && (!self.get_flag(FLAG_INTERRUPT_DISABLE) || self.waiting)
        {
            self.irq_pending = false;
            if let Some(mask) = bus.next_irq() {
                bus.acknowledge_irq(mask);
            }
            return self.handle_interrupt(bus, 0xFFFE, false);
        }

        if self.waiting {
            return 0;
        }

        let opcode = self.fetch_byte(bus);
        self.last_opcode = opcode;
        match opcode {
            // Load A
            0xA9 => {
                let value = self.fetch_byte(bus);
                self.lda(value, 2)
            }
            0xA5 => {
                let addr = self.addr_zeropage(bus);
                self.lda(Cpu::read_operand(bus, addr, true), 3)
            }
            0xA1 => {
                let addr = self.addr_indexed_indirect_x(bus);
                self.lda(bus.read(addr), 6)
            }
            0xB5 => {
                let addr = self.addr_zeropage_x(bus);
                self.lda(Cpu::read_operand(bus, addr, true), 4)
            }
            0xAD => {
                let addr = self.addr_absolute(bus);
                self.lda(bus.read(addr), 4)
            }
            0xBD => {
                let (addr, crossed) = self.addr_absolute_x(bus);
                let cycles = self.lda(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0xB9 => {
                let (addr, crossed) = self.addr_absolute_y(bus);
                let cycles = self.lda(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0xB1 => {
                let (addr, crossed) = self.addr_indirect_y(bus);
                let cycles = self.lda(bus.read(addr), 5);
                cycles + crossed as u8
            }
            0xB2 => {
                let addr = self.addr_indirect(bus);
                self.lda(bus.read(addr), 5)
            }

            // Load X
            0xA2 => {
                let value = self.fetch_byte(bus);
                self.ldx(value, 2)
            }
            0xA6 => {
                let addr = self.addr_zeropage(bus);
                self.ldx(Cpu::read_operand(bus, addr, true), 3)
            }
            0xB6 => {
                let addr = self.addr_zeropage_y(bus);
                self.ldx(Cpu::read_operand(bus, addr, true), 4)
            }
            0xAE => {
                let addr = self.addr_absolute(bus);
                self.ldx(bus.read(addr), 4)
            }
            0xBE => {
                let (addr, crossed) = self.addr_absolute_y(bus);
                let cycles = self.ldx(bus.read(addr), 4);
                cycles + crossed as u8
            }

            // Load Y
            0xA0 => {
                let value = self.fetch_byte(bus);
                self.ldy(value, 2)
            }
            0xA4 => {
                let addr = self.addr_zeropage(bus);
                self.ldy(Cpu::read_operand(bus, addr, true), 3)
            }
            0xB4 => {
                let addr = self.addr_zeropage_x(bus);
                self.ldy(Cpu::read_operand(bus, addr, true), 4)
            }
            0xAC => {
                let addr = self.addr_absolute(bus);
                self.ldy(bus.read(addr), 4)
            }
            0xBC => {
                let (addr, crossed) = self.addr_absolute_x(bus);
                let cycles = self.ldy(bus.read(addr), 4);
                cycles + crossed as u8
            }

            // Store A
            0x85 => {
                let addr = self.addr_zeropage(bus);
                Cpu::write_operand(bus, addr, self.a, true);
                3
            }
            0x81 => {
                let addr = self.addr_indexed_indirect_x(bus);
                bus.write(addr, self.a);
                6
            }
            0x95 => {
                let addr = self.addr_zeropage_x(bus);
                Cpu::write_operand(bus, addr, self.a, true);
                4
            }
            0x8D => {
                let addr = self.addr_absolute(bus);
                bus.write(addr, self.a);
                4
            }
            0x9D => {
                let (addr, _) = self.addr_absolute_x(bus);
                bus.write(addr, self.a);
                5
            }
            0x99 => {
                let (addr, _) = self.addr_absolute_y(bus);
                bus.write(addr, self.a);
                5
            }
            0x92 => {
                let addr = self.addr_indirect(bus);
                bus.write(addr, self.a);
                5
            }
            0x91 => {
                let (addr, _) = self.addr_indirect_y(bus);
                bus.write(addr, self.a);
                6
            }

            // Store X
            0x86 => {
                let addr = self.addr_zeropage(bus);
                Cpu::write_operand(bus, addr, self.x, true);
                3
            }
            0x96 => {
                let addr = self.addr_zeropage_y(bus);
                Cpu::write_operand(bus, addr, self.x, true);
                4
            }
            0x8E => {
                let addr = self.addr_absolute(bus);
                bus.write(addr, self.x);
                4
            }

            // Store Y
            0x84 => {
                let addr = self.addr_zeropage(bus);
                Cpu::write_operand(bus, addr, self.y, true);
                3
            }
            0x94 => {
                let addr = self.addr_zeropage_x(bus);
                Cpu::write_operand(bus, addr, self.y, true);
                4
            }
            0x8C => {
                let addr = self.addr_absolute(bus);
                bus.write(addr, self.y);
                4
            }

            // Arithmetic
            0x69 => {
                let value = self.fetch_byte(bus);
                self.adc(value, 2)
            }
            0x65 => {
                let addr = self.addr_zeropage(bus);
                self.adc(Cpu::read_operand(bus, addr, true), 3)
            }
            0x61 => {
                let addr = self.addr_indexed_indirect_x(bus);
                self.adc(bus.read(addr), 6)
            }
            0x75 => {
                let addr = self.addr_zeropage_x(bus);
                self.adc(Cpu::read_operand(bus, addr, true), 4)
            }
            0x6D => {
                let addr = self.addr_absolute(bus);
                self.adc(bus.read(addr), 4)
            }
            0x7D => {
                let (addr, crossed) = self.addr_absolute_x(bus);
                let cycles = self.adc(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0x79 => {
                let (addr, crossed) = self.addr_absolute_y(bus);
                let cycles = self.adc(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0x71 => {
                let (addr, crossed) = self.addr_indirect_y(bus);
                let cycles = self.adc(bus.read(addr), 5);
                cycles + crossed as u8
            }

            0xE9 | 0xEB => {
                let value = self.fetch_byte(bus);
                self.sbc(value, 2)
            }
            0xE5 => {
                let addr = self.addr_zeropage(bus);
                self.sbc(Cpu::read_operand(bus, addr, true), 3)
            }
            0xE1 => {
                let addr = self.addr_indexed_indirect_x(bus);
                self.sbc(bus.read(addr), 6)
            }
            0xF5 => {
                let addr = self.addr_zeropage_x(bus);
                self.sbc(Cpu::read_operand(bus, addr, true), 4)
            }
            0xED => {
                let addr = self.addr_absolute(bus);
                self.sbc(bus.read(addr), 4)
            }
            0xF1 => {
                let (addr, crossed) = self.addr_indirect_y(bus);
                let cycles = self.sbc(bus.read(addr), 5);
                cycles + crossed as u8
            }
            0xF2 => {
                let addr = self.addr_indirect(bus);
                self.sbc(bus.read(addr), 5)
            }
            0xFD => {
                let (addr, crossed) = self.addr_absolute_x(bus);
                let cycles = self.sbc(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0xF9 => {
                let (addr, crossed) = self.addr_absolute_y(bus);
                let cycles = self.sbc(bus.read(addr), 4);
                cycles + crossed as u8
            }

            // Logical
            0x29 => {
                let value = self.fetch_byte(bus);
                self.and(value, 2)
            }
            0x25 => {
                let addr = self.addr_zeropage(bus);
                self.and(Cpu::read_operand(bus, addr, true), 3)
            }
            0x21 => {
                let addr = self.addr_indexed_indirect_x(bus);
                self.and(bus.read(addr), 6)
            }
            0x35 => {
                let addr = self.addr_zeropage_x(bus);
                self.and(Cpu::read_operand(bus, addr, true), 4)
            }
            0x2D => {
                let addr = self.addr_absolute(bus);
                self.and(bus.read(addr), 4)
            }
            0x3D => {
                let (addr, crossed) = self.addr_absolute_x(bus);
                let cycles = self.and(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0x39 => {
                let (addr, crossed) = self.addr_absolute_y(bus);
                let cycles = self.and(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0x31 => {
                let (addr, crossed) = self.addr_indirect_y(bus);
                let cycles = self.and(bus.read(addr), 5);
                cycles + crossed as u8
            }

            0x09 => {
                let value = self.fetch_byte(bus);
                self.ora(value, 2)
            }
            0x05 => {
                let addr = self.addr_zeropage(bus);
                self.ora(Cpu::read_operand(bus, addr, true), 3)
            }
            0x01 => {
                let addr = self.addr_indexed_indirect_x(bus);
                self.ora(bus.read(addr), 6)
            }
            0x15 => {
                let addr = self.addr_zeropage_x(bus);
                self.ora(Cpu::read_operand(bus, addr, true), 4)
            }
            0x0D => {
                let addr = self.addr_absolute(bus);
                self.ora(bus.read(addr), 4)
            }
            0x1D => {
                let (addr, crossed) = self.addr_absolute_x(bus);
                let cycles = self.ora(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0x19 => {
                let (addr, crossed) = self.addr_absolute_y(bus);
                let cycles = self.ora(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0x11 => {
                let (addr, crossed) = self.addr_indirect_y(bus);
                let cycles = self.ora(bus.read(addr), 5);
                cycles + crossed as u8
            }

            0x49 => {
                let value = self.fetch_byte(bus);
                self.eor(value, 2)
            }
            0x45 => {
                let addr = self.addr_zeropage(bus);
                self.eor(Cpu::read_operand(bus, addr, true), 3)
            }
            0x41 => {
                let addr = self.addr_indexed_indirect_x(bus);
                self.eor(bus.read(addr), 6)
            }
            0x55 => {
                let addr = self.addr_zeropage_x(bus);
                self.eor(Cpu::read_operand(bus, addr, true), 4)
            }
            0x4D => {
                let addr = self.addr_absolute(bus);
                self.eor(bus.read(addr), 4)
            }
            0x5D => {
                let (addr, crossed) = self.addr_absolute_x(bus);
                let cycles = self.eor(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0x59 => {
                let (addr, crossed) = self.addr_absolute_y(bus);
                let cycles = self.eor(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0x51 => {
                let (addr, crossed) = self.addr_indirect_y(bus);
                let cycles = self.eor(bus.read(addr), 5);
                cycles + crossed as u8
            }

            // BIT tests accumulator against memory without modifying A
            0x24 => {
                let addr = self.addr_zeropage(bus);
                self.bit(Cpu::read_operand(bus, addr, true), 3)
            }
            0x34 => {
                let addr = self.addr_zeropage_x(bus);
                self.bit(Cpu::read_operand(bus, addr, true), 4)
            }
            0x2C => {
                let addr = self.addr_absolute(bus);
                self.bit(bus.read(addr), 4)
            }
            0x3C => {
                let (addr, crossed) = self.addr_absolute_x(bus);
                let cycles = self.bit(bus.read(addr), 4);
                cycles + crossed as u8
            }
            0x89 => {
                let value = self.fetch_byte(bus);
                self.bit(value, 2)
            }

            // Store zero / test and set/reset bits
            0x64 => {
                let addr = self.addr_zeropage(bus);
                self.stz(bus, addr, 3)
            }
            0x74 => {
                let addr = self.addr_zeropage_x(bus);
                self.stz(bus, addr, 4)
            }
            0x9C => {
                let addr = self.addr_absolute(bus);
                self.stz(bus, addr, 4)
            }
            0x9E => {
                let (addr, _) = self.addr_absolute_x(bus);
                self.stz(bus, addr, 5)
            }

            0x04 => {
                let addr = self.addr_zeropage(bus);
                self.tsb(bus, addr, 5)
            }
            0x0C => {
                let addr = self.addr_absolute(bus);
                self.tsb(bus, addr, 6)
            }

            0x14 => {
                let addr = self.addr_zeropage(bus);
                self.trb(bus, addr, 5)
            }
            0x1C => {
                let addr = self.addr_absolute(bus);
                self.trb(bus, addr, 6)
            }

            // Shift / rotate
            0x0A => self.asl_acc(),
            0x06 => {
                let addr = self.addr_zeropage(bus);
                self.asl_mem(bus, addr, 5)
            }
            0x16 => {
                let addr = self.addr_zeropage_x(bus);
                self.asl_mem(bus, addr, 6)
            }
            0x0E => {
                let addr = self.addr_absolute(bus);
                self.asl_mem(bus, addr, 6)
            }
            0x1E => {
                let (addr, _) = self.addr_absolute_x(bus);
                self.asl_mem(bus, addr, 7)
            }

            0x4A => self.lsr_acc(),
            0x46 => {
                let addr = self.addr_zeropage(bus);
                self.lsr_mem(bus, addr, 5)
            }
            0x56 => {
                let addr = self.addr_zeropage_x(bus);
                self.lsr_mem(bus, addr, 6)
            }
            0x4E => {
                let addr = self.addr_absolute(bus);
                self.lsr_mem(bus, addr, 6)
            }
            0x5E => {
                let (addr, _) = self.addr_absolute_x(bus);
                self.lsr_mem(bus, addr, 7)
            }

            // Increment memory
            0xE6 => {
                let addr = self.addr_zeropage(bus);
                self.inc_mem(bus, addr, 5)
            }
            0xF6 => {
                let addr = self.addr_zeropage_x(bus);
                self.inc_mem(bus, addr, 6)
            }
            0xEE => {
                let addr = self.addr_absolute(bus);
                self.inc_mem(bus, addr, 6)
            }
            0xFE => {
                let (addr, _) = self.addr_absolute_x(bus);
                self.inc_mem(bus, addr, 7)
            }
            0xC6 => {
                let addr = self.addr_zeropage(bus);
                self.dec_mem(bus, addr, 5)
            }
            0xD6 => {
                let addr = self.addr_zeropage_x(bus);
                self.dec_mem(bus, addr, 6)
            }
            0xCE => {
                let addr = self.addr_absolute(bus);
                self.dec_mem(bus, addr, 6)
            }
            0xDE => {
                let (addr, _) = self.addr_absolute_x(bus);
                self.dec_mem(bus, addr, 7)
            }

            0x2A => self.rol_acc(),
            0x26 => {
                let addr = self.addr_zeropage(bus);
                self.rol_mem(bus, addr, 5)
            }
            0x36 => {
                let addr = self.addr_zeropage_x(bus);
                self.rol_mem(bus, addr, 6)
            }
            0x2E => {
                let addr = self.addr_absolute(bus);
                self.rol_mem(bus, addr, 6)
            }
            0x3E => {
                let (addr, _) = self.addr_absolute_x(bus);
                self.rol_mem(bus, addr, 7)
            }

            0x6A => self.ror_acc(),
            0x66 => {
                let addr = self.addr_zeropage(bus);
                self.ror_mem(bus, addr, 5)
            }
            0x76 => {
                let addr = self.addr_zeropage_x(bus);
                self.ror_mem(bus, addr, 6)
            }
            0x6E => {
                let addr = self.addr_absolute(bus);
                self.ror_mem(bus, addr, 6)
            }
            0x7E => {
                let (addr, _) = self.addr_absolute_x(bus);
                self.ror_mem(bus, addr, 7)
            }
            0x7B => {
                let (addr, _) = self.addr_absolute_y(bus);
                self.rra_mem(bus, addr, 7)
            }

            // Stack pushes/pulls
            0x48 => self.pha(bus),
            0x5A => self.phy(bus),
            0xDA => self.phx(bus),
            0x08 => self.php(bus),
            0x68 => self.pla(bus),
            0x7A => self.ply(bus),
            0xFA => self.plx(bus),
            0x28 => self.plp(bus),
            0x40 => self.rti(bus),
            0xCB => self.wai(),
            0x53 => self.tam(bus),
            0x43 => self.tma(bus),
            0x07 => self.rmb(bus, 0, 5),
            0x17 => self.rmb(bus, 1, 5),
            0x27 => self.rmb(bus, 2, 5),
            0x37 => self.rmb(bus, 3, 5),
            0x47 => self.rmb(bus, 4, 5),
            0x57 => self.rmb(bus, 5, 5),
            0x67 => self.rmb(bus, 6, 5),
            0x77 => self.rmb(bus, 7, 5),
            0x87 => self.smb(bus, 0, 5),
            0x97 => self.smb(bus, 1, 5),
            0xA7 => self.smb(bus, 2, 5),
            0xB7 => self.smb(bus, 3, 5),
            0xC7 => self.smb(bus, 4, 5),
            0xD7 => self.smb(bus, 5, 5),
            0xE7 => self.smb(bus, 6, 5),
            0xF7 => self.smb(bus, 7, 5),
            0x0F => self.bbr(bus, 0),
            0x1F => self.bbr(bus, 1),
            0x2F => self.bbr(bus, 2),
            0x3F => self.bbr(bus, 3),
            0x4F => self.bbr(bus, 4),
            0x5F => self.bbr(bus, 5),
            0x6F => self.bbr(bus, 6),
            0x7F => self.bbr(bus, 7),
            0x8F => self.bbs(bus, 0),
            0x9F => self.bbs(bus, 1),
            0xAF => self.bbs(bus, 2),
            0xBF => self.bbs(bus, 3),
            0xCF => self.bbs(bus, 4),
            0xDF => self.bbs(bus, 5),
            0xEF => self.bbs(bus, 6),
            0xFF => self.bbs(bus, 7),
            0x83 => self.tst_zero_page(bus),
            0xA3 => self.tst_zero_page_x(bus),
            0x93 => self.tst_absolute(bus),
            0xB3 => self.tst_absolute_x(bus),
            0x03 => self.st_port(bus, 0),
            0x13 => self.st_port(bus, 1),
            0x23 => self.st_port(bus, 2),
            0xDB => self.stp(),
            0x73 => self.exec_block_transfer(bus, BlockMode::Tii),
            0xC3 => self.exec_block_transfer(bus, BlockMode::Tdd),
            0xD3 => self.exec_block_transfer(bus, BlockMode::Tin),
            0xE3 => self.exec_block_transfer(bus, BlockMode::Tia),
            0xF3 => self.exec_block_transfer(bus, BlockMode::Tai),

            // Increment / Decrement
            0xE8 => self.inx(),
            0xC8 => self.iny(),
            0x1A => self.ina(),
            0xCA => self.dex(),
            0x88 => self.dey(),
            0x3A => self.dea(),

            // Comparisons
            0xC9 => {
                let value = self.fetch_byte(bus);
                self.cmp(value, self.a, 2)
            }
            0xC5 => {
                let addr = self.addr_zeropage(bus);
                self.cmp(Cpu::read_operand(bus, addr, true), self.a, 3)
            }
            0xC1 => {
                let addr = self.addr_indexed_indirect_x(bus);
                self.cmp(bus.read(addr), self.a, 6)
            }
            0xD5 => {
                let addr = self.addr_zeropage_x(bus);
                self.cmp(bus.read(addr), self.a, 4)
            }
            0xCD => {
                let addr = self.addr_absolute(bus);
                self.cmp(bus.read(addr), self.a, 4)
            }
            0xDD => {
                let (addr, crossed) = self.addr_absolute_x(bus);
                let cycles = self.cmp(bus.read(addr), self.a, 4);
                cycles + crossed as u8
            }
            0xD9 => {
                let (addr, crossed) = self.addr_absolute_y(bus);
                let cycles = self.cmp(bus.read(addr), self.a, 4);
                cycles + crossed as u8
            }
            0xD1 => {
                let (addr, crossed) = self.addr_indirect_y(bus);
                let cycles = self.cmp(bus.read(addr), self.a, 5);
                cycles + crossed as u8
            }

            0xE0 => {
                let value = self.fetch_byte(bus);
                self.cmp(value, self.x, 2)
            }
            0xE4 => {
                let addr = self.addr_zeropage(bus);
                self.cmp(Cpu::read_operand(bus, addr, true), self.x, 3)
            }
            0xEC => {
                let addr = self.addr_absolute(bus);
                self.cmp(bus.read(addr), self.x, 4)
            }

            0xC0 => {
                let value = self.fetch_byte(bus);
                self.cmp(value, self.y, 2)
            }
            0xC4 => {
                let addr = self.addr_zeropage(bus);
                self.cmp(Cpu::read_operand(bus, addr, true), self.y, 3)
            }
            0xCC => {
                let addr = self.addr_absolute(bus);
                self.cmp(bus.read(addr), self.y, 4)
            }

            // Branches
            0x90 => self.branch(bus, !self.get_flag(FLAG_CARRY)),
            0xB0 => self.branch(bus, self.get_flag(FLAG_CARRY)),
            0xF0 => self.branch(bus, self.get_flag(FLAG_ZERO)),
            0x30 => self.branch(bus, self.get_flag(FLAG_NEGATIVE)),
            0xD0 => self.branch(bus, !self.get_flag(FLAG_ZERO)),
            0x10 => self.branch(bus, !self.get_flag(FLAG_NEGATIVE)),
            0x50 => self.branch(bus, !self.get_flag(FLAG_OVERFLOW)),
            0x70 => self.branch(bus, self.get_flag(FLAG_OVERFLOW)),
            0x80 => self.branch(bus, true),

            // Status
            0x18 => {
                self.set_flag(FLAG_CARRY, false);
                2
            }
            0x38 => {
                self.set_flag(FLAG_CARRY, true);
                2
            }
            0x58 => {
                self.set_flag(FLAG_INTERRUPT_DISABLE, false);
                2
            }
            0x78 => {
                self.set_flag(FLAG_INTERRUPT_DISABLE, true);
                2
            }
            0xB8 => {
                self.set_flag(FLAG_OVERFLOW, false);
                2
            }
            0xD8 => {
                self.set_flag(FLAG_DECIMAL, false);
                2
            }
            0xF8 => {
                self.set_flag(FLAG_DECIMAL, true);
                2
            }
            0xF4 => self.set_t_flag(),
            0xD4 => self.csh(),
            0x54 => self.csl(),

            // Transfers
            0x62 => self.cla(),
            0x82 => self.clx(),
            0xC2 => self.cly(),
            0xAA => self.tax(),
            0xA8 => self.tay(),
            0x8A => self.txa(),
            0x98 => self.tya(),
            0xBA => self.tsx(),
            0x9A => self.txs(),
            0x22 => self.sax(),
            0x42 => self.say(),
            0x02 => self.sxy(),

            // Stack / control
            0x44 => self.bsr(bus),
            0x20 => self.jsr(bus),
            0x4C => self.jmp_absolute(bus),
            0x6C => self.jmp_indirect(bus),
            0x7C => self.jmp_indirect_indexed(bus),
            0x60 => self.rts(bus),
            0x00 => self.brk(bus),
            0xEA => 2, // NOP

            _ => {
                let mprs: [u8; 8] = std::array::from_fn(|i| bus.mpr(i));
                let pc = self.pc.wrapping_sub(1);
                let status = self.status;
                panic!(
                    "Unimplemented opcode {opcode:#04X} at PC={pc:#06X} (A={:#04X} X={:#04X} Y={:#04X} SP={:#04X} P={status:#04X} MPR={:?})",
                    self.a, self.x, self.y, self.sp, mprs
                )
            }
        }
    }

    fn lda(&mut self, value: u8, cycles: u8) -> u8 {
        self.a = value;
        self.update_zero_and_negative(self.a);
        cycles
    }

    fn ldx(&mut self, value: u8, cycles: u8) -> u8 {
        self.x = value;
        self.update_zero_and_negative(self.x);
        cycles
    }

    fn ldy(&mut self, value: u8, cycles: u8) -> u8 {
        self.y = value;
        self.update_zero_and_negative(self.y);
        cycles
    }

    fn adc(&mut self, value: u8, cycles: u8) -> u8 {
        let carry = if self.get_flag(FLAG_CARRY) { 1 } else { 0 };
        let sum = self.a as u16 + value as u16 + carry as u16;
        let result = sum as u8;

        self.set_flag(FLAG_CARRY, sum > 0xFF);
        self.set_flag(
            FLAG_OVERFLOW,
            (!(self.a ^ value) & (self.a ^ result) & 0x80) != 0,
        );

        self.a = result;
        self.update_zero_and_negative(self.a);
        cycles
    }

    fn sbc(&mut self, value: u8, cycles: u8) -> u8 {
        let carry = if self.get_flag(FLAG_CARRY) { 1 } else { 0 };
        let subtrahend = value as u16 + (1 - carry) as u16;
        let minuend = self.a as u16;
        let result = minuend.wrapping_sub(subtrahend);
        let result_byte = result as u8;

        self.set_flag(FLAG_CARRY, minuend >= subtrahend);
        self.set_flag(
            FLAG_OVERFLOW,
            ((self.a ^ result_byte) & (self.a ^ value) & 0x80) != 0,
        );

        self.a = result_byte;
        self.update_zero_and_negative(self.a);
        cycles
    }

    fn and(&mut self, value: u8, cycles: u8) -> u8 {
        self.a &= value;
        self.update_zero_and_negative(self.a);
        cycles
    }

    fn ora(&mut self, value: u8, cycles: u8) -> u8 {
        self.a |= value;
        self.update_zero_and_negative(self.a);
        cycles
    }

    fn eor(&mut self, value: u8, cycles: u8) -> u8 {
        self.a ^= value;
        self.update_zero_and_negative(self.a);
        cycles
    }

    fn asl_acc(&mut self) -> u8 {
        let carry = (self.a & 0x80) != 0;
        self.a = self.a.wrapping_shl(1);
        self.set_flag(FLAG_CARRY, carry);
        self.update_zero_and_negative(self.a);
        2
    }

    fn asl_mem(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        let value = Cpu::read_operand(bus, addr, zero_page);
        let carry = (value & 0x80) != 0;
        let result = value.wrapping_shl(1);
        Cpu::write_operand(bus, addr, result, zero_page);
        self.set_flag(FLAG_CARRY, carry);
        self.update_zero_and_negative(result);
        cycles
    }

    fn lsr_acc(&mut self) -> u8 {
        let carry = (self.a & 0x01) != 0;
        self.a >>= 1;
        self.set_flag(FLAG_CARRY, carry);
        self.update_zero_and_negative(self.a);
        2
    }

    fn lsr_mem(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        let value = Cpu::read_operand(bus, addr, zero_page);
        let carry = (value & 0x01) != 0;
        let result = value >> 1;
        Cpu::write_operand(bus, addr, result, zero_page);
        self.set_flag(FLAG_CARRY, carry);
        self.update_zero_and_negative(result);
        cycles
    }

    fn rol_acc(&mut self) -> u8 {
        let carry_in = if self.get_flag(FLAG_CARRY) { 1 } else { 0 };
        let carry_out = (self.a & 0x80) != 0;
        self.a = (self.a << 1) | carry_in;
        self.set_flag(FLAG_CARRY, carry_out);
        self.update_zero_and_negative(self.a);
        2
    }

    fn rol_mem(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        let value = Cpu::read_operand(bus, addr, zero_page);
        let carry_in = if self.get_flag(FLAG_CARRY) { 1 } else { 0 };
        let carry_out = (value & 0x80) != 0;
        let result = (value << 1) | carry_in;
        Cpu::write_operand(bus, addr, result, zero_page);
        self.set_flag(FLAG_CARRY, carry_out);
        self.update_zero_and_negative(result);
        cycles
    }

    fn ror_acc(&mut self) -> u8 {
        let carry_in = if self.get_flag(FLAG_CARRY) { 1 } else { 0 };
        let carry_out = (self.a & 0x01) != 0;
        self.a = (self.a >> 1) | (carry_in << 7);
        self.set_flag(FLAG_CARRY, carry_out);
        self.update_zero_and_negative(self.a);
        2
    }

    fn ror_mem(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        let value = Cpu::read_operand(bus, addr, zero_page);
        let carry_in = if self.get_flag(FLAG_CARRY) { 1 } else { 0 };
        let carry_out = (value & 0x01) != 0;
        let result = (value >> 1) | (carry_in << 7);
        Cpu::write_operand(bus, addr, result, zero_page);
        self.set_flag(FLAG_CARRY, carry_out);
        self.update_zero_and_negative(result);
        cycles
    }

    fn rra_mem(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        let value = Cpu::read_operand(bus, addr, zero_page);
        let carry_in = if self.get_flag(FLAG_CARRY) { 1 } else { 0 };
        let carry_out = (value & 0x01) != 0;
        let rotated = (value >> 1) | (carry_in << 7);
        Cpu::write_operand(bus, addr, rotated, zero_page);
        self.set_flag(FLAG_CARRY, carry_out);
        let _ = self.adc(rotated, 0);
        cycles
    }

    fn inc_mem(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        let value = Cpu::read_operand(bus, addr, zero_page).wrapping_add(1);
        Cpu::write_operand(bus, addr, value, zero_page);
        self.update_zero_and_negative(value);
        cycles
    }

    fn dec_mem(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        let value = Cpu::read_operand(bus, addr, zero_page).wrapping_sub(1);
        Cpu::write_operand(bus, addr, value, zero_page);
        self.update_zero_and_negative(value);
        cycles
    }

    fn pha(&mut self, bus: &mut Bus) -> u8 {
        self.push_byte(bus, self.a);
        3
    }

    fn phy(&mut self, bus: &mut Bus) -> u8 {
        self.push_byte(bus, self.y);
        3
    }

    fn phx(&mut self, bus: &mut Bus) -> u8 {
        self.push_byte(bus, self.x);
        3
    }

    fn php(&mut self, bus: &mut Bus) -> u8 {
        let value = self.status | FLAG_BREAK | FLAG_T;
        self.push_byte(bus, value);
        3
    }

    fn pla(&mut self, bus: &mut Bus) -> u8 {
        let value = self.pop_byte(bus);
        self.a = value;
        self.update_zero_and_negative(self.a);
        4
    }

    fn ply(&mut self, bus: &mut Bus) -> u8 {
        self.y = self.pop_byte(bus);
        self.update_zero_and_negative(self.y);
        4
    }

    fn plx(&mut self, bus: &mut Bus) -> u8 {
        self.x = self.pop_byte(bus);
        self.update_zero_and_negative(self.x);
        4
    }

    fn plp(&mut self, bus: &mut Bus) -> u8 {
        let value = self.pop_byte(bus);
        let restored = value | FLAG_T;
        // Break flag behaves as stored on the stack; keep bit 4 from popped value.
        self.status = restored;
        self.halted = false;
        self.waiting = false;
        4
    }

    fn rti(&mut self, bus: &mut Bus) -> u8 {
        let status = self.pop_byte(bus) | FLAG_T;
        self.status = status;
        let lo = self.pop_byte(bus) as u16;
        let hi = self.pop_byte(bus) as u16;
        self.pc = (hi << 8) | lo;
        self.halted = false;
        self.waiting = false;
        6
    }

    fn wai(&mut self) -> u8 {
        self.waiting = true;
        self.set_flag(FLAG_INTERRUPT_DISABLE, true);
        3
    }

    fn tam(&mut self, bus: &mut Bus) -> u8 {
        let mask = self.fetch_byte(bus);
        for i in 0..8 {
            if mask & (1 << i) != 0 {
                bus.set_mpr(i, self.a);
            }
        }
        5
    }

    fn tma(&mut self, bus: &mut Bus) -> u8 {
        let mask = self.fetch_byte(bus);
        let mut value = self.a;
        for i in 0..8 {
            if mask & (1 << i) != 0 {
                value = bus.mpr(i);
                break;
            }
        }
        self.a = value;
        self.update_zero_and_negative(self.a);
        4
    }

    fn rmb(&mut self, bus: &mut Bus, bit: u8, cycles: u8) -> u8 {
        let addr = self.fetch_byte(bus);
        let mut value = bus.read_zero_page(addr);
        value &= !(1 << bit);
        bus.write_zero_page(addr, value);
        cycles
    }

    fn smb(&mut self, bus: &mut Bus, bit: u8, cycles: u8) -> u8 {
        let addr = self.fetch_byte(bus);
        let mut value = bus.read_zero_page(addr);
        value |= 1 << bit;
        bus.write_zero_page(addr, value);
        cycles
    }

    fn bbr(&mut self, bus: &mut Bus, bit: u8) -> u8 {
        self.branch_on_bit(bus, bit, false)
    }

    fn bbs(&mut self, bus: &mut Bus, bit: u8) -> u8 {
        self.branch_on_bit(bus, bit, true)
    }

    fn tst_zero_page(&mut self, bus: &mut Bus) -> u8 {
        let mask = self.fetch_byte(bus);
        let addr = self.fetch_byte(bus);
        let value = bus.read_zero_page(addr);
        self.tst(mask, value);
        7
    }

    fn tst_zero_page_x(&mut self, bus: &mut Bus) -> u8 {
        let mask = self.fetch_byte(bus);
        let addr = self.fetch_byte(bus).wrapping_add(self.x);
        let value = bus.read_zero_page(addr);
        self.tst(mask, value);
        7
    }

    fn tst_absolute(&mut self, bus: &mut Bus) -> u8 {
        let mask = self.fetch_byte(bus);
        let addr = self.fetch_word(bus);
        let value = bus.read(addr);
        self.tst(mask, value);
        8
    }

    fn tst_absolute_x(&mut self, bus: &mut Bus) -> u8 {
        let mask = self.fetch_byte(bus);
        let base = self.fetch_word(bus);
        let addr = base.wrapping_add(self.x as u16);
        let value = bus.read(addr);
        self.tst(mask, value);
        8
    }

    fn st_port(&mut self, bus: &mut Bus, port: usize) -> u8 {
        let value = self.fetch_byte(bus);
        bus.write_st_port(port, value);
        5
    }

    fn stp(&mut self) -> u8 {
        self.halted = true;
        3
    }

    fn inx(&mut self) -> u8 {
        self.x = self.x.wrapping_add(1);
        self.update_zero_and_negative(self.x);
        2
    }

    fn iny(&mut self) -> u8 {
        self.y = self.y.wrapping_add(1);
        self.update_zero_and_negative(self.y);
        2
    }

    fn ina(&mut self) -> u8 {
        self.a = self.a.wrapping_add(1);
        self.update_zero_and_negative(self.a);
        2
    }

    fn dex(&mut self) -> u8 {
        self.x = self.x.wrapping_sub(1);
        self.update_zero_and_negative(self.x);
        2
    }

    fn dey(&mut self) -> u8 {
        self.y = self.y.wrapping_sub(1);
        self.update_zero_and_negative(self.y);
        2
    }

    fn dea(&mut self) -> u8 {
        self.a = self.a.wrapping_sub(1);
        self.update_zero_and_negative(self.a);
        2
    }

    fn cmp(&mut self, value: u8, register: u8, cycles: u8) -> u8 {
        let result = register.wrapping_sub(value);
        self.set_flag(FLAG_CARRY, register >= value);
        self.update_zero_and_negative(result);
        cycles
    }

    fn bit(&mut self, value: u8, cycles: u8) -> u8 {
        self.set_flag(FLAG_ZERO, (self.a & value) == 0);
        self.set_flag(FLAG_NEGATIVE, value & 0x80 != 0);
        self.set_flag(FLAG_OVERFLOW, value & 0x40 != 0);
        cycles
    }

    fn stz(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        Cpu::write_operand(bus, addr, 0, zero_page);
        cycles
    }

    fn tsb(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        let value = Cpu::read_operand(bus, addr, zero_page);
        let test = self.a & value;
        self.set_flag(FLAG_ZERO, test == 0);
        let result = value | self.a;
        Cpu::write_operand(bus, addr, result, zero_page);
        cycles
    }

    fn trb(&mut self, bus: &mut Bus, addr: u16, cycles: u8) -> u8 {
        let zero_page = addr < 0x0100;
        let value = Cpu::read_operand(bus, addr, zero_page);
        let test = self.a & value;
        self.set_flag(FLAG_ZERO, test == 0);
        let result = value & !self.a;
        Cpu::write_operand(bus, addr, result, zero_page);
        cycles
    }

    fn handle_interrupt(&mut self, bus: &mut Bus, vector: u16, set_break: bool) -> u8 {
        let pc = self.pc;
        self.push_byte(bus, (pc >> 8) as u8);
        self.push_byte(bus, pc as u8);
        let mut status = self.status | FLAG_T;
        if set_break {
            status |= FLAG_BREAK;
        } else {
            status &= !FLAG_BREAK;
        }
        self.push_byte(bus, status);
        self.set_flag(FLAG_INTERRUPT_DISABLE, true);
        self.pc = bus.read_u16(vector);
        self.waiting = false;
        self.halted = false;
        7
    }

    fn exec_block_transfer(&mut self, bus: &mut Bus, mode: BlockMode) -> u8 {
        let (source, dest, length) = self.fetch_block_params(bus);

        // Hardware pushes A, X, Y to the stack before the transfer.citeturn1search0turn1search1
        self.push_byte(bus, self.a);
        self.push_byte(bus, self.x);
        self.push_byte(bus, self.y);

        let cycles = self.block_transfer(bus, source, dest, length, mode);

        self.y = self.pop_byte(bus);
        self.x = self.pop_byte(bus);
        self.a = self.pop_byte(bus);
        cycles
    }

    fn block_transfer(
        &mut self,
        bus: &mut Bus,
        source: u16,
        dest: u16,
        length: u32,
        mode: BlockMode,
    ) -> u8 {
        let mut remaining = length;
        let mut src_ptr = source;
        let mut dest_ptr = dest;
        let mut dest_alt: u16 = 0;
        let mut src_alt: u16 = 0;
        let mut cycles: u64 = 17;
        #[cfg(feature = "trace_hw_writes")]
        {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static LOGGED: AtomicUsize = AtomicUsize::new(0);
            let n = LOGGED.fetch_add(1, Ordering::Relaxed);
            if n < 4 {
                eprintln!(
                    "BLOCK DMA start mode={:?} src={:04X} dest={:04X} len={}",
                    mode, source, dest, length
                );
            }
        }
        #[cfg(feature = "trace_hw_writes")]
        {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static LOGGED: AtomicUsize = AtomicUsize::new(0);
            if LOGGED.fetch_add(1, Ordering::Relaxed) < 5 {
                eprintln!(
                    "BLOCK DMA mode={:?} src={:04X} dest={:04X} len={}",
                    mode, source, dest, length
                );
            }
        }

        while remaining > 0 {
            match mode {
                BlockMode::Tii => {
                    let value = bus.read(src_ptr);
                    bus.write(dest_ptr, value);
                    #[cfg(feature = "trace_hw_writes")]
                    if remaining >= length.saturating_sub(4) {
                        eprintln!(
                            "  BLK {:?} first bytes src={:04X} val={:02X} dest={:04X}",
                            mode, src_ptr, value, dest_ptr
                        );
                    }
                    if cfg!(debug_assertions) {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static ANY_LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = ANY_LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 32 {
                            eprintln!(
                                "BLK xfer {:?} src={:04X} val={:02X} dest={:04X}",
                                mode, src_ptr, value, dest_ptr
                            );
                        }
                    }
                    #[cfg(feature = "trace_hw_writes")]
                    if (dest_ptr & 0x1FFE) == 0x0402 {
                        eprintln!("BLOCK DMA hit VCE {:04X} <= {:02X}", dest_ptr, value);
                    }
                    if cfg!(debug_assertions) && (dest_ptr & 0x1FFE) == 0x0402 {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 64 {
                            eprintln!(
                                "BLK VCE write mode={:?} src={:04X} val={:02X} dest={:04X}",
                                mode, src_ptr, value, dest_ptr
                            );
                        }
                    }
                    src_ptr = src_ptr.wrapping_add(1);
                    dest_ptr = dest_ptr.wrapping_add(1);
                }
                BlockMode::Tdd => {
                    let value = bus.read(src_ptr);
                    bus.write(dest_ptr, value);
                    #[cfg(feature = "trace_hw_writes")]
                    if remaining >= length.saturating_sub(4) {
                        eprintln!(
                            "  BLK {:?} first bytes src={:04X} val={:02X} dest={:04X}",
                            mode, src_ptr, value, dest_ptr
                        );
                    }
                    if cfg!(debug_assertions) {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static ANY_LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = ANY_LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 32 {
                            eprintln!(
                                "BLK xfer {:?} src={:04X} val={:02X} dest={:04X}",
                                mode, src_ptr, value, dest_ptr
                            );
                        }
                    }
                    #[cfg(feature = "trace_hw_writes")]
                    if (dest_ptr & 0x1FFE) == 0x0402 {
                        eprintln!("BLOCK DMA hit VCE {:04X} <= {:02X}", dest_ptr, value);
                    }
                    if cfg!(debug_assertions) && (dest_ptr & 0x1FFE) == 0x0402 {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 64 {
                            eprintln!(
                                "BLK VCE write mode={:?} src={:04X} val={:02X} dest={:04X}",
                                mode, src_ptr, value, dest_ptr
                            );
                        }
                    }
                    src_ptr = src_ptr.wrapping_sub(1);
                    dest_ptr = dest_ptr.wrapping_sub(1);
                }
                BlockMode::Tin => {
                    let value = bus.read(src_ptr);
                    bus.write(dest_ptr, value);
                    #[cfg(feature = "trace_hw_writes")]
                    if remaining >= length.saturating_sub(4) {
                        eprintln!(
                            "  BLK {:?} first bytes src={:04X} val={:02X} dest={:04X}",
                            mode, src_ptr, value, dest_ptr
                        );
                    }
                    if cfg!(debug_assertions) {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static ANY_LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = ANY_LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 32 {
                            eprintln!(
                                "BLK xfer {:?} src={:04X} val={:02X} dest={:04X}",
                                mode, src_ptr, value, dest_ptr
                            );
                        }
                    }
                    #[cfg(feature = "trace_hw_writes")]
                    if (dest_ptr & 0x1FFE) == 0x0402 {
                        eprintln!("BLOCK DMA hit VCE {:04X} <= {:02X}", dest_ptr, value);
                    }
                    if cfg!(debug_assertions) && (dest_ptr & 0x1FFE) == 0x0402 {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 64 {
                            eprintln!(
                                "BLK VCE write mode={:?} src={:04X} val={:02X} dest={:04X}",
                                mode, src_ptr, value, dest_ptr
                            );
                        }
                    }
                    src_ptr = src_ptr.wrapping_add(1);
                }
                BlockMode::Tia => {
                    let value = bus.read(src_ptr);
                    let target = dest.wrapping_add(dest_alt);
                    bus.write(target, value);
                    #[cfg(feature = "trace_hw_writes")]
                    if remaining >= length.saturating_sub(4) {
                        eprintln!(
                            "  BLK {:?} first bytes src={:04X} val={:02X} dest={:04X}",
                            mode, src_ptr, value, target
                        );
                    }
                    if cfg!(debug_assertions) {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static ANY_LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = ANY_LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 32 {
                            eprintln!(
                                "BLK xfer {:?} src={:04X} val={:02X} dest={:04X}",
                                mode, src_ptr, value, target
                            );
                        }
                    }
                    #[cfg(feature = "trace_hw_writes")]
                    if (target & 0x1FFE) == 0x0402 {
                        eprintln!("BLOCK DMA hit VCE {:04X} <= {:02X}", target, value);
                    }
                    if cfg!(debug_assertions) && (target & 0x1FFE) == 0x0402 {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 64 {
                            eprintln!(
                                "BLK VCE write mode={:?} src={:04X} val={:02X} dest={:04X}",
                                mode, src_ptr, value, target
                            );
                        }
                    }
                    src_ptr = src_ptr.wrapping_add(1);
                    dest_alt ^= 1;
                }
                BlockMode::Tai => {
                    let addr = source.wrapping_add(src_alt);
                    let value = bus.read(addr);
                    bus.write(dest_ptr, value);
                    #[cfg(feature = "trace_hw_writes")]
                    if remaining >= length.saturating_sub(4) {
                        eprintln!(
                            "  BLK {:?} first bytes src={:04X} val={:02X} dest={:04X}",
                            mode, addr, value, dest_ptr
                        );
                    }
                    if cfg!(debug_assertions) {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static ANY_LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = ANY_LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 32 {
                            eprintln!(
                                "BLK xfer {:?} src={:04X} val={:02X} dest={:04X}",
                                mode, addr, value, dest_ptr
                            );
                        }
                    }
                    #[cfg(feature = "trace_hw_writes")]
                    if (dest_ptr & 0x1FFE) == 0x0402 {
                        eprintln!("BLOCK DMA hit VCE {:04X} <= {:02X}", dest_ptr, value);
                    }
                    if cfg!(debug_assertions) && (dest_ptr & 0x1FFE) == 0x0402 {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static LOGGED: AtomicUsize = AtomicUsize::new(0);
                        let n = LOGGED.fetch_add(1, Ordering::Relaxed);
                        if n < 64 {
                            eprintln!(
                                "BLK VCE write mode={:?} src={:04X} val={:02X} dest={:04X}",
                                mode, addr, value, dest_ptr
                            );
                        }
                    }
                    dest_ptr = dest_ptr.wrapping_add(1);
                    src_alt ^= 1;
                }
            }

            remaining -= 1;
            cycles += 6;
        }

        self.waiting = false;
        (cycles & 0xFF) as u8
    }

    fn branch_on_bit(&mut self, bus: &mut Bus, bit: u8, branch_if_set: bool) -> u8 {
        let zp_addr = self.fetch_byte(bus);
        let value = bus.read_zero_page(zp_addr);
        let offset = self.fetch_byte(bus) as i8;
        let bit_set = (value & (1 << bit)) != 0;
        let condition = if branch_if_set { bit_set } else { !bit_set };

        let mut cycles = 5u8;
        if condition {
            let prev_pc = self.pc;
            let target = ((self.pc as i32 + offset as i32) as u32) as u16;
            self.pc = target;
            cycles += 2;
            if Cpu::page_crossed(prev_pc, target) {
                cycles += 1;
            }
        }

        cycles
    }

    fn tst(&mut self, mask: u8, value: u8) {
        let result = mask & value;
        self.set_flag(FLAG_ZERO, result == 0);
        self.set_flag(FLAG_NEGATIVE, result & 0x80 != 0);
        self.set_flag(FLAG_OVERFLOW, result & 0x40 != 0);
    }

    fn cla(&mut self) -> u8 {
        self.a = 0;
        self.update_zero_and_negative(self.a);
        2
    }

    fn clx(&mut self) -> u8 {
        self.x = 0;
        self.update_zero_and_negative(self.x);
        2
    }

    fn cly(&mut self) -> u8 {
        self.y = 0;
        self.update_zero_and_negative(self.y);
        2
    }

    fn sax(&mut self) -> u8 {
        std::mem::swap(&mut self.a, &mut self.x);
        self.update_zero_and_negative(self.a);
        3
    }

    fn say(&mut self) -> u8 {
        std::mem::swap(&mut self.a, &mut self.y);
        self.update_zero_and_negative(self.a);
        3
    }

    fn sxy(&mut self) -> u8 {
        std::mem::swap(&mut self.x, &mut self.y);
        self.update_zero_and_negative(self.x);
        3
    }

    fn set_t_flag(&mut self) -> u8 {
        self.set_flag(FLAG_T, true);
        2
    }

    fn csh(&mut self) -> u8 {
        self.clock_high_speed = true;
        self.set_flag(FLAG_T, false);
        3
    }

    fn csl(&mut self) -> u8 {
        self.clock_high_speed = false;
        self.set_flag(FLAG_T, false);
        3
    }

    fn bsr(&mut self, bus: &mut Bus) -> u8 {
        let offset = self.fetch_byte(bus) as i8;
        let return_addr = self.pc;
        self.push_byte(bus, (return_addr >> 8) as u8);
        self.push_byte(bus, return_addr as u8);
        self.pc = ((self.pc as i32 + offset as i32) as u32) as u16;
        8
    }

    fn branch(&mut self, bus: &mut Bus, condition: bool) -> u8 {
        let offset = self.fetch_byte(bus) as i8;
        if condition {
            let prev_pc = self.pc;
            self.pc = ((self.pc as i32 + offset as i32) as u32) as u16;
            let mut cycles = 3u8;
            if Cpu::page_crossed(prev_pc, self.pc) {
                cycles += 1;
            }
            cycles
        } else {
            2
        }
    }

    fn tax(&mut self) -> u8 {
        self.x = self.a;
        self.update_zero_and_negative(self.x);
        2
    }

    fn tay(&mut self) -> u8 {
        self.y = self.a;
        self.update_zero_and_negative(self.y);
        2
    }

    fn txa(&mut self) -> u8 {
        self.a = self.x;
        self.update_zero_and_negative(self.a);
        2
    }

    fn tya(&mut self) -> u8 {
        self.a = self.y;
        self.update_zero_and_negative(self.a);
        2
    }

    fn tsx(&mut self) -> u8 {
        self.x = self.sp;
        self.update_zero_and_negative(self.x);
        2
    }

    fn txs(&mut self) -> u8 {
        self.sp = self.x;
        2
    }

    fn jsr(&mut self, bus: &mut Bus) -> u8 {
        let addr = self.addr_absolute(bus);
        let return_addr = self.pc.wrapping_sub(1);
        self.push_byte(bus, (return_addr >> 8) as u8);
        self.push_byte(bus, return_addr as u8);
        self.pc = addr;
        6
    }

    fn rts(&mut self, bus: &mut Bus) -> u8 {
        let lo = self.pop_byte(bus) as u16;
        let hi = self.pop_byte(bus) as u16;
        self.pc = ((hi << 8) | lo).wrapping_add(1);
        6
    }

    fn brk(&mut self, bus: &mut Bus) -> u8 {
        // BRK consumes an extra byte, so advance PC to skip the padding.
        self.pc = self.pc.wrapping_add(1);
        let vector = bus.read_u16(0xFFFE);
        if vector == 0x0000 {
            // No BRK vector installed: emulate development ROMs/tests by halting.
            self.halted = true;
            return 7;
        }

        // Defer to the standard interrupt sequence so cartridge handlers observe
        // the pushed PC/status bytes just like on hardware.
        self.handle_interrupt(bus, 0xFFFE, true)
    }

    fn jmp_absolute(&mut self, bus: &mut Bus) -> u8 {
        let target = self.fetch_word(bus);
        self.pc = target;
        3
    }

    fn jmp_indirect(&mut self, bus: &mut Bus) -> u8 {
        let ptr = self.fetch_word(bus);
        let lo = bus.read(ptr);
        let hi_addr = (ptr & 0xFF00) | ((ptr + 1) & 0x00FF);
        let hi = bus.read(hi_addr);
        self.pc = ((hi as u16) << 8) | lo as u16;
        5
    }

    fn jmp_indirect_indexed(&mut self, bus: &mut Bus) -> u8 {
        let base = self.fetch_word(bus);
        let ptr = base.wrapping_add(self.x as u16);
        let lo = bus.read(ptr);
        let hi = bus.read(ptr.wrapping_add(1));
        self.pc = ((hi as u16) << 8) | lo as u16;
        6
    }

    fn addr_zeropage(&mut self, bus: &mut Bus) -> u16 {
        self.fetch_byte(bus) as u16
    }

    fn addr_zeropage_x(&mut self, bus: &mut Bus) -> u16 {
        self.fetch_byte(bus).wrapping_add(self.x) as u16
    }

    fn addr_zeropage_y(&mut self, bus: &mut Bus) -> u16 {
        self.fetch_byte(bus).wrapping_add(self.y) as u16
    }

    fn addr_absolute(&mut self, bus: &mut Bus) -> u16 {
        self.fetch_word(bus)
    }

    fn addr_absolute_x(&mut self, bus: &mut Bus) -> (u16, bool) {
        let base = self.fetch_word(bus);
        let addr = base.wrapping_add(self.x as u16);
        (addr, Cpu::page_crossed(base, addr))
    }

    fn addr_absolute_y(&mut self, bus: &mut Bus) -> (u16, bool) {
        let base = self.fetch_word(bus);
        let addr = base.wrapping_add(self.y as u16);
        (addr, Cpu::page_crossed(base, addr))
    }

    fn addr_indirect(&mut self, bus: &mut Bus) -> u16 {
        let ptr = self.fetch_byte(bus);
        Cpu::read_zero_page_word(bus, ptr)
    }

    fn addr_indexed_indirect_x(&mut self, bus: &mut Bus) -> u16 {
        let base = self.fetch_byte(bus).wrapping_add(self.x);
        Cpu::read_zero_page_word(bus, base)
    }

    fn addr_indirect_y(&mut self, bus: &mut Bus) -> (u16, bool) {
        let base_ptr = self.fetch_byte(bus);
        let base = Cpu::read_zero_page_word(bus, base_ptr);
        let addr = base.wrapping_add(self.y as u16);
        (addr, Cpu::page_crossed(base, addr))
    }

    fn fetch_byte(&mut self, bus: &mut Bus) -> u8 {
        let value = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        value
    }

    fn fetch_word(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.fetch_byte(bus) as u16;
        let hi = self.fetch_byte(bus) as u16;
        (hi << 8) | lo
    }

    fn fetch_block_params(&mut self, bus: &mut Bus) -> (u16, u16, u32) {
        let src_lo = self.fetch_byte(bus) as u16;
        let src_hi = self.fetch_byte(bus) as u16;
        let dst_lo = self.fetch_byte(bus) as u16;
        let dst_hi = self.fetch_byte(bus) as u16;
        let len_lo = self.fetch_byte(bus) as u16;
        let len_hi = self.fetch_byte(bus) as u16;
        let source = (src_hi << 8) | src_lo;
        let dest = (dst_hi << 8) | dst_lo;
        let length_raw = (len_hi << 8) | len_lo;
        let length = if length_raw == 0 {
            0x1_0000
        } else {
            length_raw as u32
        };
        (source, dest, length)
    }

    fn read_zero_page_word(bus: &mut Bus, addr: u8) -> u16 {
        let lo = bus.read_zero_page(addr) as u16;
        let hi = bus.read_zero_page(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    #[inline]
    fn read_operand(bus: &mut Bus, addr: u16, zero_page: bool) -> u8 {
        if zero_page {
            bus.read_zero_page(addr as u8)
        } else {
            bus.read(addr)
        }
    }

    #[inline]
    fn write_operand(bus: &mut Bus, addr: u16, value: u8, zero_page: bool) {
        if zero_page {
            bus.write_zero_page(addr as u8, value);
        } else {
            bus.write(addr, value);
        }
    }

    fn push_byte(&mut self, bus: &mut Bus, value: u8) {
        let addr = 0x0100 | self.sp as u16;
        bus.stack_write(addr, value);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pop_byte(&mut self, bus: &mut Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        let addr = 0x0100 | self.sp as u16;
        bus.stack_read(addr)
    }

    fn update_zero_and_negative(&mut self, value: u8) {
        self.set_flag(FLAG_ZERO, value == 0);
        self.set_flag(FLAG_NEGATIVE, value & 0x80 != 0);
    }

    fn set_flag(&mut self, flag: u8, value: bool) {
        if value {
            self.status |= flag;
        } else {
            self.status &= !flag;
        }
    }

    fn get_flag(&self, flag: u8) -> bool {
        self.status & flag != 0
    }

    fn page_crossed(a: u16, b: u16) -> bool {
        (a & 0xFF00) != (b & 0xFF00)
    }

    #[allow(dead_code)]
    pub fn flag(&self, flag: u8) -> bool {
        self.get_flag(flag)
    }

    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    pub fn last_opcode(&self) -> u8 {
        self.last_opcode
    }
}

#[derive(Clone, Copy, Debug)]
enum BlockMode {
    Tii,
    Tin,
    Tdd,
    Tia,
    Tai,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{IRQ_REQUEST_IRQ1, IRQ_REQUEST_IRQ2, IRQ_REQUEST_TIMER, PAGE_SIZE};

    fn setup_cpu_with_program(program: &[u8]) -> (Cpu, Bus) {
        let mut bus = Bus::new();
        bus.load(0x8000, program);
        bus.write_u16(0xFFFC, 0x8000);

        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        (cpu, bus)
    }

    #[test]
    fn adc_handles_carry_and_overflow() {
        let program = [0x69, 0x01, 0x69, 0x80, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.a = 0x7F;

        cpu.step(&mut bus); // ADC #$01 => 0x80
        assert_eq!(cpu.a, 0x80);
        assert!(cpu.flag(FLAG_NEGATIVE));
        assert!(cpu.flag(FLAG_OVERFLOW));

        cpu.step(&mut bus); // ADC #$80 => 0x00 with carry
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.flag(FLAG_CARRY));
    }

    #[test]
    fn branch_taken_adds_cycles_and_adjusts_pc() {
        // BNE +2 to skip BRK, then immediate BRK to halt.
        let program = [0xD0, 0x02, 0x00, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        cpu.status &= !FLAG_ZERO;
        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 3); // branch taken same page
        assert_eq!(cpu.pc, 0x8004);
    }

    #[test]
    fn jsr_and_rts_round_trip() {
        // JSR $8004 ; LDA #$42 ; RTS ; BRK
        let program = [0x20, 0x04, 0x80, 0x00, 0xA9, 0x42, 0x60, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        cpu.step(&mut bus); // JSR
        assert_eq!(cpu.pc, 0x8004);
        assert_eq!(bus.read(0x01FC), 0x02);
        assert_eq!(bus.read(0x01FD), 0x80);
        cpu.step(&mut bus); // LDA
        assert_eq!(cpu.a, 0x42);
        cpu.step(&mut bus); // RTS
        assert_eq!(cpu.pc, 0x8003); // return to byte after JSR operand
    }

    #[test]
    fn lda_indexed_indirect_x_reads_correct_value() {
        let program = [0xA1, 0x10, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.x = 0x05;
        bus.write(0x0015, 0x00);
        bus.write(0x0016, 0x90);
        bus.write(0x9000, 0xAB);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cpu.a, 0xAB);
        assert_eq!(cycles, 6);
    }

    #[test]
    fn lda_indirect_y_page_cross_adds_cycle() {
        let program = [0xB1, 0x20, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0020, 0xFF);
        bus.write(0x0021, 0x80);
        bus.write(0x8100, 0x34);
        cpu.y = 0x01;

        let cycles = cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x34);
        assert_eq!(cycles, 6);
    }

    #[test]
    fn sta_indirect_y_stores_value() {
        let program = [0x91, 0x30, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.a = 0x77;
        cpu.y = 0x05;
        bus.write(0x0030, 0x00);
        bus.write(0x0031, 0x44);

        let cycles = cpu.step(&mut bus);
        assert_eq!(bus.read(0x4405), 0x77);
        assert_eq!(cycles, 6);
    }

    #[test]
    fn bit_immediate_updates_flags_without_touching_accumulator() {
        let program = [0x89, 0xC0, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.a = 0xFF;

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 2);
        assert_eq!(cpu.a, 0xFF);
        assert!(!cpu.flag(FLAG_ZERO));
        assert!(cpu.flag(FLAG_NEGATIVE));
        assert!(cpu.flag(FLAG_OVERFLOW));
    }

    #[test]
    fn bit_zeropage_sets_zero_when_mask_clears_bits() {
        let program = [0x24, 0x40, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.a = 0x10;
        bus.write(0x0040, 0x04);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 3);
        assert!(cpu.flag(FLAG_ZERO));
        assert!(!cpu.flag(FLAG_NEGATIVE));
        assert!(!cpu.flag(FLAG_OVERFLOW));
    }

    #[test]
    fn asl_accumulator_sets_carry() {
        let program = [0x0A, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.a = 0x81;

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 2);
        assert_eq!(cpu.a, 0x02);
        assert!(cpu.flag(FLAG_CARRY));
        assert!(!cpu.flag(FLAG_NEGATIVE));
        assert!(!cpu.flag(FLAG_ZERO));
    }

    #[test]
    fn ror_zeropage_rotates_through_carry() {
        let program = [0x66, 0x10, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.status |= FLAG_CARRY;
        bus.write(0x0010, 0x02);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 5);
        assert_eq!(bus.read(0x0010), 0x81);
        assert!(!cpu.flag(FLAG_CARRY));
        assert!(cpu.flag(FLAG_NEGATIVE));
        assert!(!cpu.flag(FLAG_ZERO));
    }

    #[test]
    fn rra_absolute_y_rotates_and_adds() {
        let program = [0x7B, 0x00, 0x90, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.a = 0x10;
        cpu.y = 0x05;
        cpu.status |= FLAG_CARRY;
        bus.write(0x9005, 0x04);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 7);
        assert_eq!(bus.read(0x9005), 0x82);
        assert_eq!(cpu.a, 0x92);
        assert!(!cpu.flag(FLAG_CARRY));
        assert!(cpu.flag(FLAG_NEGATIVE));
        assert!(!cpu.flag(FLAG_ZERO));
    }

    #[test]
    fn pha_pla_round_trip() {
        let program = [0xA9, 0x12, 0x48, 0xA9, 0x00, 0x68, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        cpu.step(&mut bus); // LDA #$12
        assert_eq!(cpu.a, 0x12);

        cpu.step(&mut bus); // PHA
        assert_eq!(bus.read(0x01FD), 0x12);
        assert_eq!(cpu.sp, 0xFC);

        cpu.step(&mut bus); // LDA #$00
        assert_eq!(cpu.a, 0x00);

        cpu.step(&mut bus); // PLA
        assert_eq!(cpu.a, 0x12);
        assert_eq!(cpu.sp, 0xFD);
        assert!(!cpu.flag(FLAG_ZERO));
        assert!(!cpu.flag(FLAG_NEGATIVE));
    }

    #[test]
    fn php_pushes_status_with_break_bit() {
        let program = [0x08, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.status = FLAG_CARRY;

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 3);
        assert_eq!(cpu.sp, 0xFC);
        let pushed = bus.read(0x01FD);
        assert_eq!(pushed, FLAG_CARRY | FLAG_BREAK | FLAG_T);
    }

    #[test]
    fn plp_restores_flags_from_stack() {
        let program = [0x28, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.push_byte(&mut bus, FLAG_NEGATIVE | FLAG_BREAK);
        cpu.status = 0;

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 4);
        assert!(cpu.flag(FLAG_NEGATIVE));
        assert!(cpu.flag(FLAG_BREAK));
        assert!(cpu.flag(FLAG_T));
    }

    #[test]
    fn stz_zeroes_memory_without_touching_a() {
        let program = [0xA9, 0xFF, 0x64, 0x10, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        cpu.step(&mut bus); // LDA #$FF
        cpu.step(&mut bus); // STZ $10

        assert_eq!(cpu.a, 0xFF);
        assert_eq!(bus.read(0x0010), 0x00);
    }

    #[test]
    fn tsb_sets_bits_and_updates_zero_flag() {
        let program = [0xA9, 0x0F, 0x04, 0x20, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0020, 0xF3);

        cpu.step(&mut bus); // LDA #$0F
        cpu.step(&mut bus); // TSB $20

        assert_eq!(bus.read(0x0020), 0xFF);
        assert!(!cpu.flag(FLAG_ZERO));
    }

    #[test]
    fn trb_sets_zero_flag_when_no_overlap() {
        let program = [0xA9, 0xF0, 0x14, 0x30, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0030, 0x0F);

        cpu.step(&mut bus); // LDA #$F0
        cpu.step(&mut bus); // TRB $30 (no overlap)
        assert_eq!(bus.read(0x0030), 0x0F);
        assert!(cpu.flag(FLAG_ZERO));
    }

    #[test]
    fn trb_clears_bits_when_overlap_exists() {
        let program = [0xA9, 0xF0, 0x14, 0x30, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0030, 0xF3);

        cpu.step(&mut bus); // LDA #$F0
        cpu.step(&mut bus); // TRB $30 (overlap)

        assert_eq!(bus.read(0x0030), 0x03);
        assert!(!cpu.flag(FLAG_ZERO));
    }

    #[test]
    fn tii_transfers_incrementing_addresses() {
        let program = [0x73, 0x00, 0x90, 0x00, 0x40, 0x03, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x9000, 0x11);
        bus.write(0x9001, 0x22);
        bus.write(0x9002, 0x33);

        cpu.step(&mut bus);

        assert_eq!(bus.read(0x4000), 0x11);
        assert_eq!(bus.read(0x4001), 0x22);
        assert_eq!(bus.read(0x4002), 0x33);
        assert_eq!(cpu.sp, 0xFD);
    }

    #[test]
    fn tin_leaves_destination_fixed() {
        let program = [0xD3, 0x00, 0x90, 0x00, 0x40, 0x02, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x9000, 0xAA);
        bus.write(0x9001, 0xBB);

        cpu.step(&mut bus);

        assert_eq!(bus.read(0x4000), 0xBB);
        assert_eq!(bus.read(0x4001), 0x00);
        assert_eq!(cpu.sp, 0xFD);
    }

    #[test]
    fn tia_alternates_destination_bytes() {
        let program = [0xE3, 0x00, 0x90, 0x00, 0x40, 0x02, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x9000, 0x5A);
        bus.write(0x9001, 0xC3);

        cpu.step(&mut bus);

        assert_eq!(bus.read(0x4000), 0x5A);
        assert_eq!(bus.read(0x4001), 0xC3);
        assert_eq!(cpu.sp, 0xFD);
    }

    #[test]
    fn tdd_transfers_decrementing_addresses() {
        let program = [0xC3, 0x02, 0x90, 0x02, 0x40, 0x03, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x9002, 0x11);
        bus.write(0x9001, 0x22);
        bus.write(0x9000, 0x33);

        cpu.step(&mut bus);

        assert_eq!(bus.read(0x4002), 0x11);
        assert_eq!(bus.read(0x4001), 0x22);
        assert_eq!(bus.read(0x4000), 0x33);
        assert_eq!(cpu.sp, 0xFD);
    }

    #[test]
    fn tai_reads_alternating_source_bytes() {
        let program = [0xF3, 0x00, 0x90, 0x00, 0x30, 0x04, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x9000, 0xAA);
        bus.write(0x9001, 0xBB);
        bus.write(0x9002, 0xCC);

        cpu.step(&mut bus);

        assert_eq!(bus.read(0x3000), 0xAA);
        assert_eq!(bus.read(0x3001), 0xBB);
        assert_eq!(bus.read(0x3002), 0xAA);
        assert_eq!(bus.read(0x3003), 0xBB);
        assert_eq!(cpu.sp, 0xFD);
    }

    #[test]
    fn block_moves_treat_zero_length_as_65536_iterations() {
        let program = [0x73, 0x00, 0x90, 0x00, 0x20, 0x00, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x9000, 0x42);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 17);
        assert_eq!(cpu.sp, 0xFD);
    }

    #[test]
    fn ina_dea_adjust_accumulator_and_flags() {
        let program = [0x1A, 0x1A, 0x3A, 0x3A, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.a = 0x7F;
        cpu.step(&mut bus); // INA -> 0x80
        assert_eq!(cpu.a, 0x80);
        assert!(cpu.flag(FLAG_NEGATIVE));
        assert!(!cpu.flag(FLAG_ZERO));

        cpu.step(&mut bus); // INA -> 0x81
        assert_eq!(cpu.a, 0x81);

        cpu.step(&mut bus); // DEA -> 0x80
        assert_eq!(cpu.a, 0x80);
        assert!(cpu.flag(FLAG_NEGATIVE));

        cpu.step(&mut bus); // DEA -> 0x7F
        assert_eq!(cpu.a, 0x7F);
        assert!(!cpu.flag(FLAG_NEGATIVE));
        assert!(!cpu.flag(FLAG_ZERO));
    }

    #[test]
    fn phx_plx_and_phy_ply_round_trip_registers() {
        let program = [0xDA, 0xFA, 0x5A, 0x7A, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.x = 0x42;
        cpu.y = 0x80;
        cpu.step(&mut bus); // PHX (push at $01FD, sp -> 0xFC)
        assert_eq!(bus.read(0x01FD), 0x42);
        cpu.x = 0x00;
        cpu.step(&mut bus); // PLX
        assert_eq!(cpu.x, 0x42);
        assert!(!cpu.flag(FLAG_ZERO));

        cpu.step(&mut bus); // PHY (rewrites $01FD, sp -> 0xFC)
        assert_eq!(bus.read(0x01FD), 0x80);
        cpu.y = 0x00;
        cpu.step(&mut bus); // PLY
        assert_eq!(cpu.y, 0x80);
        assert!(cpu.flag(FLAG_NEGATIVE));
    }

    #[test]
    fn sta_zero_page_indirect_stores_value() {
        let program = [0xA9, 0x5A, 0x92, 0x10, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0010, 0x00);
        bus.write(0x0011, 0xC0);

        while !cpu.halted {
            cpu.step(&mut bus);
        }

        assert_eq!(bus.read(0xC000), 0x5A);
    }

    #[test]
    fn jmp_absolute_sets_pc() {
        let program = [0x4C, 0x05, 0x80, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x8005);
    }

    #[test]
    fn jmp_indirect_wraps_page() {
        let program = [0x6C, 0xFF, 0x82, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x82FF, 0x34);
        // 6502 page-wrapped high byte fetch uses low-page wrap around.
        bus.write(0x8200, 0x12);
        cpu.step(&mut bus);
        assert_eq!(cpu.pc, 0x1234);
    }

    #[test]
    fn jmp_indirect_indexed_uses_offset() {
        let program = [0xA2, 0x02, 0x7C, 0x00, 0x90, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.load(0x9002, &[0x78, 0x56]);
        cpu.step(&mut bus); // LDX #$02
        cpu.step(&mut bus); // JMP ($9000,X)
        assert_eq!(cpu.pc, 0x5678);
    }

    #[test]
    fn tam_updates_mprs_and_remaps_page() {
        let program = [
            0xA9, 0xF8, // LDA #$F8 (internal RAM window)
            0x53, 0x01, // TAM #$01 (MPR0)
            0xA9, 0x5A, // LDA #$5A
            0x8D, 0x00, 0x00, // STA $0000 -> maps to page selected by MPR0
            0x00,
        ];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        while !cpu.halted {
            cpu.step(&mut bus);
        }

        assert_eq!(bus.mpr(0), 0xF8);
        assert_eq!(bus.read(0x0000), 0x5A);
    }

    #[test]
    fn tma_reads_from_selected_mpr() {
        let program = [0xA9, 0x00, 0x43, 0x08, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.set_mpr(3, 0x44);

        while !cpu.halted {
            cpu.step(&mut bus);
        }

        assert_eq!(cpu.a, 0x44);
        assert!(!cpu.flag(FLAG_ZERO));
        assert!(!cpu.flag(FLAG_NEGATIVE));
    }

    #[test]
    fn rmb_clears_bit_in_zero_page() {
        let program = [0xA9, 0xFF, 0x85, 0x10, 0x07, 0x10, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        cpu.step(&mut bus); // LDA #$FF
        cpu.step(&mut bus); // STA $10
        cpu.step(&mut bus); // RMB0 $10

        assert_eq!(bus.read(0x0010), 0xFE);
    }

    #[test]
    fn smb_sets_bit_in_zero_page() {
        let program = [0xA9, 0x00, 0x85, 0x11, 0xC7, 0x11, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        cpu.step(&mut bus); // LDA #$00
        cpu.step(&mut bus); // STA $11
        cpu.step(&mut bus); // SMB4 $11

        assert_eq!(bus.read(0x0011), 0x10);
    }

    #[test]
    fn bbr_branches_when_bit_reset() {
        let program = [0x0F, 0x10, 0x01, 0xEA, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0010, 0x00);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 7);
        assert_eq!(cpu.pc, 0x8004);
        cpu.step(&mut bus);
        assert!(cpu.halted);
    }

    #[test]
    fn bbs_skips_when_bit_clear() {
        let program = [0x8F, 0x10, 0x01, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0010, 0x00);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 5);
        assert_eq!(cpu.pc, 0x8003);
        cpu.step(&mut bus);
        assert!(cpu.halted);
    }

    #[test]
    fn bbs_branches_when_bit_set() {
        let program = [0x8F, 0x10, 0x01, 0xEA, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0010, 0x01);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 7);
        assert_eq!(cpu.pc, 0x8004);
        cpu.step(&mut bus);
        assert!(cpu.halted);
    }

    #[test]
    fn bbr_taken_cross_page_costs_extra_cycle() {
        let mut program = vec![0xEA; 0xFC];
        program.extend([0x0F, 0x10, 0x02, 0xEA, 0x00, 0x00]);
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0010, 0x00);
        cpu.pc = 0x80FC;

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 8);
        assert_eq!(cpu.pc, 0x8101);
        assert_eq!(bus.read(0x0010), 0x00);
        cpu.step(&mut bus);
        assert!(cpu.halted);
    }

    #[test]
    fn bbs_taken_cross_page_costs_extra_cycle() {
        let mut program = vec![0xEA; 0xFC];
        program.extend([0x8F, 0x10, 0x02, 0xEA, 0x00, 0x00]);
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0010, 0x01);
        cpu.pc = 0x80FC;

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 8);
        assert_eq!(cpu.pc, 0x8101);
        assert_eq!(bus.read(0x0010), 0x01);
        cpu.step(&mut bus);
        assert!(cpu.halted);
    }

    #[test]
    fn tst_zp_sets_flags_based_on_mask_and_value() {
        let program = [0x83, 0xF0, 0x20, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x0020, 0xF0);
        cpu.a = 0x00; // TST does not use A but ensure non-zero

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 7);
        assert!(!cpu.flag(FLAG_ZERO));
        assert!(cpu.flag(FLAG_NEGATIVE));
        assert!(cpu.flag(FLAG_OVERFLOW));
    }

    #[test]
    fn tst_abs_sets_zero_when_masked_out() {
        let program = [0x93, 0x0F, 0x00, 0x90, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write(0x9000, 0xF0);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 8);
        assert!(cpu.flag(FLAG_ZERO));
        assert!(!cpu.flag(FLAG_NEGATIVE));
        assert!(!cpu.flag(FLAG_OVERFLOW));
    }

    #[test]
    fn cla_clx_cly_clear_registers() {
        let program = [0x62, 0x82, 0xC2, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.a = 0xFF;
        cpu.x = 0x80;
        cpu.y = 0x01;

        cpu.step(&mut bus);
        assert_eq!(cpu.a, 0);
        assert!(cpu.flag(FLAG_ZERO));

        cpu.step(&mut bus);
        assert_eq!(cpu.x, 0);
        assert!(cpu.flag(FLAG_ZERO));

        cpu.step(&mut bus);
        assert_eq!(cpu.y, 0);
        assert!(cpu.flag(FLAG_ZERO));
    }

    #[test]
    fn sax_say_sxy_swap_registers() {
        let program = [0x22, 0x42, 0x02, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.a = 0x12;
        cpu.x = 0x34;
        cpu.y = 0x56;

        cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x34);
        assert_eq!(cpu.x, 0x12);

        cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x56);
        assert_eq!(cpu.y, 0x34);

        cpu.step(&mut bus);
        assert_eq!(cpu.x, 0x34);
        assert_eq!(cpu.y, 0x12);
    }

    #[test]
    fn set_and_clock_switch_instructions() {
        let program = [0xF4, 0xD4, 0x54, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        cpu.set_flag(FLAG_T, false);

        cpu.step(&mut bus);
        assert!(cpu.flag(FLAG_T));

        cpu.step(&mut bus);
        assert!(!cpu.flag(FLAG_T));
        assert!(cpu.clock_high_speed);

        cpu.step(&mut bus);
        assert!(!cpu.flag(FLAG_T));
        assert!(!cpu.clock_high_speed);
    }

    #[test]
    fn bsr_pushes_return_address() {
        let program = [0x44, 0x02, 0x00, 0xEA, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 8);
        assert_eq!(cpu.pc, 0x8004);
        assert_eq!(cpu.sp, 0xFB);
        let lo = bus.read(0x01FC);
        let hi = bus.read(0x01FD);
        assert_eq!(lo, 0x02);
        assert_eq!(hi, 0x80);
    }

    #[test]
    fn st_ports_write_immediate_values() {
        let program = [0x03, 0xAA, 0x13, 0xBB, 0x23, 0xCC, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        cpu.step(&mut bus);
        cpu.step(&mut bus);
        cpu.step(&mut bus);

        assert_eq!(bus.st_port(0), 0xAA);
        assert_eq!(bus.st_port(1), 0xBB);
        assert_eq!(bus.st_port(2), 0xCC);
    }

    #[test]
    fn stp_halts_cpu() {
        let program = [0xDB, 0xEA];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 3);
        assert!(cpu.halted);

        let next_cycles = cpu.step(&mut bus);
        assert_eq!(next_cycles, 0);
    }

    #[test]
    fn writing_mpr_via_memory_updates_mapping() {
        let program = [0xA9, 0x08, 0x8D, 0x80, 0xFF, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);

        cpu.step(&mut bus); // LDA #$08
        cpu.step(&mut bus); // STA $FF80

        assert_eq!(bus.mpr(0), 0x08);

        bus.load_rom_image(vec![0x11; PAGE_SIZE * 4]);

        assert_eq!(bus.read(0x0000), 0x11);
    }

    #[test]
    fn wai_pauses_until_irq() {
        let program = [0xCB, 0xEA, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write_u16(0xFFFE, 0x9000);
        bus.load(0x9000, &[0xEA, 0x00]);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 3);
        assert_eq!(cpu.pc, 0x8001);

        let idle_cycles = cpu.step(&mut bus);
        assert_eq!(idle_cycles, 0);
        assert_eq!(cpu.pc, 0x8001);

        bus.tick(64, true);
        bus.raise_irq(IRQ_REQUEST_TIMER);
        let irq_cycles = cpu.step(&mut bus);
        assert_eq!(irq_cycles, 7);
        assert_eq!(cpu.pc, 0x9000);
    }

    #[test]
    fn irq_and_rti_restore_state() {
        let program = [0xEA, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write_u16(0xFFFE, 0x9000);
        bus.load(0x9000, &[0x40, 0x00]);

        cpu.status = FLAG_CARRY;
        bus.raise_irq(IRQ_REQUEST_TIMER);
        let irq_cycles = cpu.step(&mut bus);
        assert_eq!(irq_cycles, 7);
        assert_eq!(cpu.pc, 0x9000);
        assert_eq!(cpu.sp, 0xFA);

        // Stack order: status pushed last at current SP+1 (0x01FB)
        assert_eq!(bus.read(0x01FB), FLAG_CARRY | FLAG_T);
        assert_eq!(bus.read(0x01FC), 0x00); // PCL
        assert_eq!(bus.read(0x01FD), 0x80); // PCH

        let rti_cycles = cpu.step(&mut bus);
        assert_eq!(rti_cycles, 6);
        assert_eq!(cpu.pc, 0x8000);
        assert_eq!(cpu.sp, 0xFD);
        assert!(cpu.flag(FLAG_CARRY));
    }

    #[test]
    fn multiple_irq_sources_preserve_lower_priority() {
        let program = [0xEA, 0x00];
        let (mut cpu, mut bus) = setup_cpu_with_program(&program);
        bus.write_u16(0xFFFE, 0x9000);
        bus.load(0x9000, &[0x40, 0x00]);

        cpu.status &= !FLAG_INTERRUPT_DISABLE;
        bus.raise_irq(IRQ_REQUEST_IRQ1 | IRQ_REQUEST_IRQ2 | IRQ_REQUEST_TIMER);

        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 7);
        assert_eq!(cpu.pc, 0x9000);
        assert_eq!(
            bus.pending_interrupts() & IRQ_REQUEST_IRQ1,
            IRQ_REQUEST_IRQ1
        );
        assert_eq!(
            bus.pending_interrupts() & IRQ_REQUEST_IRQ2,
            IRQ_REQUEST_IRQ2
        );

        let _ = cpu.step(&mut bus); // RTI from timer handler
        assert_eq!(
            bus.pending_interrupts() & IRQ_REQUEST_IRQ1,
            IRQ_REQUEST_IRQ1
        );
        assert_eq!(
            bus.pending_interrupts() & IRQ_REQUEST_IRQ2,
            IRQ_REQUEST_IRQ2
        );

        let cycles = cpu.step(&mut bus); // service IRQ1
        assert_eq!(cycles, 7);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
        assert_eq!(
            bus.pending_interrupts() & IRQ_REQUEST_IRQ2,
            IRQ_REQUEST_IRQ2
        );

        let _ = cpu.step(&mut bus); // RTI from IRQ1 handler
        let cycles = cpu.step(&mut bus); // service IRQ2
        assert_eq!(cycles, 7);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ2, 0);
    }
}
