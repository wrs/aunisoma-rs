use defmt::{panic, println};
use embassy_stm32::pac::FLASH;

pub fn get_user_bytes() -> (u8, u8) {
    return (FLASH.obr().read().data0(), FLASH.obr().read().data1());
}

pub fn write_user_bytes(data0: u8, data1: u8) {
    unlock();
    ob_unlock();
    ob_erase();
    ob_write_data_bytes(data0, data1);
    ob_lock();
    lock();
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
    println!("set optpg");
    FLASH.cr().modify(|w| w.set_optpg(true));
    println!("write data0");
    write_option_word(OB_DATA_ADDRESS_DATA0, data0 as u16);
    println!("write data1");
    write_option_word(OB_DATA_ADDRESS_DATA1, data1 as u16);
    wait_for_flash_idle();
    println!("set optpg false");
    FLASH.cr().modify(|w| w.set_optpg(false));
}

fn write_option_word(address: *mut u16, value: u16) {
    println!("writing {:x} to {:x}", value, address);
    unsafe {
        core::ptr::write_volatile(address, value);
    }
    wait_for_flash_idle();
    let read_value = unsafe { core::ptr::read_volatile(address) };
    println!("read {:x} from {:x}", read_value, address);
    let expected_value = (!value << 8) | value;
    if read_value != expected_value {
        println!("expected {:x} but got {:x}", expected_value, read_value);
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
