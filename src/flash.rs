use core::sync::atomic::{AtomicBool, Ordering};

use crate::{Mode, boot, comm::CommMode};
use bitfield::bitfield;
use defmt::{Format, debug, info, panic};
use embassy_stm32::pac::FLASH;

// The option bytes register is only read from flash at power-up, so we cache
// the current values in .noinit RAM.

#[unsafe(link_section = ".noinit")]
static mut CACHED_USER_BYTES: UserBytes = UserBytes {
    id: 0,
    data1: Data1(0),
};

static CACHED_USER_BYTES_LOCK: AtomicBool = AtomicBool::new(false);

fn with_cached_user_bytes<F, R>(f: F) -> R
where
    F: FnOnce(&'static mut UserBytes) -> R,
{
    #[allow(static_mut_refs)]
    if CACHED_USER_BYTES_LOCK
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        let result = f(unsafe { &mut CACHED_USER_BYTES });
        CACHED_USER_BYTES_LOCK.store(false, Ordering::SeqCst);
        result
    } else {
        panic!("cached_user_bytes already in use");
    }
}

pub fn init_user_configuration() {
    if boot::is_warm_boot() {
        info!("warm boot");
    } else {
        unsafe { CACHED_USER_BYTES = UserBytes::get() };
        info!("cold boot");
    }
    with_cached_user_bytes(|user_bytes| info!("user bytes {:?}", user_bytes));
}

pub fn get_my_id() -> u8 {
    with_cached_user_bytes(|user_bytes| user_bytes.get_id())
}

pub fn get_default_mode() -> Mode {
    with_cached_user_bytes(|user_bytes| {
        Mode::try_from(user_bytes.default_mode()).unwrap_or(Mode::Panel)
    })
}

pub fn set_default_mode(mode: Mode) {
    with_cached_user_bytes(|user_bytes| user_bytes.set_default_mode(mode.into()));
}

pub fn get_comm_mode() -> CommMode {
    with_cached_user_bytes(|user_bytes| {
        CommMode::try_from(user_bytes.comm_mode()).unwrap_or(CommMode::Radio)
    })
}

pub fn set_comm_mode(mode: CommMode) {
    with_cached_user_bytes(|user_bytes| user_bytes.set_comm_mode(mode.into()));
}

// I'd rather use bitfield-struct, but it's generating defmt stuff that
// won't compile, despite defmt=false.

bitfield! {
    #[derive(Clone, Copy)]
    struct Data1(u8);
    u8;
    default_mode, set_default_mode: 1, 0;  // bits 0-1 for default mode
    comm_mode, set_comm_mode: 3, 2;       // bit 2-3 for comm mode
}

/// Assigns meaning to the 2 bytes of EEPROM user data on the STM32F1.
///
/// This deals in raw values. The get_ and set_ functions above translate
/// to/from the enums.
///
struct UserBytes {
    id: u8,
    data1: Data1,
}

impl Format for UserBytes {
    fn format(&self, fmt: defmt::Formatter<'_>) {
        defmt::write!(
            fmt,
            "UserBytes(id={}, default_mode={} ",
            self.id,
            self.data1.default_mode(),
        );
        if let Ok(mode) = Mode::try_from(self.data1.default_mode()) {
            defmt::write!(fmt, "({:?})", mode);
        } else {
            defmt::write!(fmt, "(invalid)");
        }
        defmt::write!(fmt, ", comm_mode={}", self.data1.comm_mode());
        if let Ok(mode) = CommMode::try_from(self.data1.comm_mode()) {
            defmt::write!(fmt, "({:?})", mode);
        } else {
            defmt::write!(fmt, "(invalid)");
        }
        defmt::write!(fmt, ")");
    }
}

impl UserBytes {
    pub fn get() -> Self {
        let id = FLASH.obr().read().data0();
        let mut data1 = Data1(FLASH.obr().read().data1());

        // Clean up the possibly uninitialized data1
        if Mode::try_from(data1.default_mode()).is_err() {
            defmt::warn!("default mode invalid, setting to Panel");
            data1.set_default_mode(Mode::Panel.into());
        }
        if CommMode::try_from(data1.comm_mode()).is_err() {
            defmt::warn!("comm mode invalid, setting to Radio");
            data1.set_comm_mode(CommMode::Radio.into());
        }

        let result = Self { id, data1 };
        debug!("Read from flash: {:?}", &result);
        result
    }

    pub fn get_id(&self) -> u8 {
        self.id
    }

    // There is no set_id() because we set the ID once per board to match
    // the number written on it.

    pub fn default_mode(&self) -> u8 {
        self.data1.default_mode()
    }

    pub fn set_default_mode(&mut self, mode: u8) {
        if mode > 3 {
            panic!("invalid default mode");
        }
        self.data1.set_default_mode(mode);
        self.write();
    }

    pub fn comm_mode(&self) -> u8 {
        self.data1.comm_mode()
    }

    pub fn set_comm_mode(&mut self, mode: u8) {
        if mode > 3 {
            panic!("invalid comm mode");
        }
        self.data1.set_comm_mode(mode);
        self.write();
    }

    pub fn write(&self) {
        debug!("writing {:?}", self);
        unlock();
        ob_unlock();
        ob_erase();
        ob_write_data_bytes(self.id, self.data1.0);
        ob_lock();
        lock();
    }
}

fn unlock() {
    if FLASH.cr().read().lock() {
        FLASH.keyr().write_value(0x45670123);
        FLASH.keyr().write_value(0xCDEF89AB);
    }
    if FLASH.cr().read().lock() {
        panic!("flash didn't unlock");
    }
}

fn ob_unlock() {
    FLASH.optkeyr().write_value(0x45670123);
    FLASH.optkeyr().write_value(0xCDEF89AB);
    if !FLASH.cr().read().optwre() {
        panic!("OB didn't unlock");
    }
}

// TODO: These addresses are for STM32F103C8. I couldn't find option bytes
// support in embassy-stm32. Maybe submit a PR.

const OB_RDP_ADDRESS: *mut u16 = 0x1FFFF800 as *mut u16;
const OB_DATA_ADDRESS_DATA0: *mut u16 = 0x1FFFF804 as *mut u16;
const OB_DATA_ADDRESS_DATA1: *mut u16 = 0x1FFFF806 as *mut u16;

// Assumes there's no read protection, and that we don't want
// any option bytes to be set, so we can just erase them all
// and write only the user data bytes.

fn ob_erase() {
    let rdprt = FLASH.obr().read().rdprt();

    wait_for_flash_idle();
    FLASH.cr().modify(|w| w.set_opter(true));
    FLASH.cr().modify(|w| w.set_strt(true));
    wait_for_flash_idle();
    FLASH.cr().modify(|w| w.set_opter(false));

    FLASH.cr().modify(|w| w.set_optpg(true));
    unsafe {
        core::ptr::write_volatile(OB_RDP_ADDRESS, if rdprt { 0x0000 } else { 0x00a5 });
    }
    wait_for_flash_idle();
    FLASH.cr().modify(|w| w.set_optpg(false));
}

fn ob_write_data_bytes(data0: u8, data1: u8) {
    wait_for_flash_idle();
    FLASH.cr().modify(|w| w.set_optpg(true));
    write_option_word(OB_DATA_ADDRESS_DATA0, data0 as u16);
    write_option_word(OB_DATA_ADDRESS_DATA1, data1 as u16);
    wait_for_flash_idle();
    FLASH.cr().modify(|w| w.set_optpg(false));
}

fn write_option_word(address: *mut u16, value: u16) {
    debug!("writing {:x} to {:x}", value, address);
    unsafe {
        core::ptr::write_volatile(address, value);
    }
    wait_for_flash_idle();
    let read_value = unsafe { core::ptr::read_volatile(address) };
    debug!("read {:x} from {:x}", read_value, address);
    let expected_value = (!value << 8) | value;
    if read_value != expected_value {
        debug!("expected {:x} but got {:x}", expected_value, read_value);
        panic!("flash write failed");
    }
}

fn ob_lock() {
    FLASH.cr().modify(|w| w.set_optwre(false));
}

fn lock() {
    FLASH.cr().modify(|w| w.set_lock(true));
}

fn wait_for_flash_idle() {
    while FLASH.sr().read().bsy() {}
    if FLASH.sr().read().eop() {
        FLASH.sr().modify(|w| w.set_eop(false));
    }
    if FLASH.sr().read().wrprterr() {
        panic!("flash wrprterr");
    }
    if FLASH.sr().read().pgerr() {
        cortex_m::asm::bkpt();
        panic!("flash pgerr");
    }
    if FLASH.obr().read().opterr() {
        panic!("flash opterr");
    }
}
