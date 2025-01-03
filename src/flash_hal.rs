use embassy_stm32::pac::FLASH;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use embassy_sync::mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use critical_section;

/// Flash HAL error codes
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum HalError {
    Timeout,
    WriteProtection,
    Programming,
    OptionByte,
}

/// Flash process type definition
#[derive(Debug)]
pub struct FlashProcess {
    procedure: FlashProcedure,
    address: u32,
    data: u64,
    data_remaining: u8,
    error_code: Option<HalError>,
}

#[derive(Debug)]
enum FlashProcedure {
    None,
    ProgramHalfWord,
    ProgramWord,
    ProgramDoubleWord,
    PageErase,
    MassErase,
}

/// Program types
#[derive(Debug, Copy, Clone)]
pub enum ProgramType {
    HalfWord,
    Word,
    DoubleWord,
}

/// Flash timeout value in ms
const FLASH_TIMEOUT_VALUE: u32 = 50_000;

/// Flash key constants
const FLASH_KEY1: u32 = 0x45670123;
const FLASH_KEY2: u32 = 0xCDEF89AB;
const FLASH_OPTKEY1: u32 = 0x45670123;
const FLASH_OPTKEY2: u32 = 0xCDEF89AB;

/// Add these static variables at the top level
static FLASH_PROCESS: Mutex<CriticalSectionRawMutex, FlashProcess> = Mutex::new(FlashProcess {
    procedure: FlashProcedure::None,
    address: 0,
    data: 0,
    data_remaining: 0,
    error_code: None,
});

/// Callback function type for flash operations
pub type FlashCallback = fn(u32);

static ERROR_CALLBACK: Mutex<CriticalSectionRawMutex, Option<FlashCallback>> = Mutex::new(None);
static END_OF_OP_CALLBACK: Mutex<CriticalSectionRawMutex, Option<FlashCallback>> = Mutex::new(None);

/// Set the error callback function
pub fn set_error_callback(callback: FlashCallback) {
    critical_section::with(|cs| {
        *ERROR_CALLBACK.borrow_ref_mut(cs) = Some(callback);
    });
}

/// Set the end of operation callback function
pub fn set_end_of_op_callback(callback: FlashCallback) {
    critical_section::with(|cs| {
        *END_OF_OP_CALLBACK.borrow_ref_mut(cs) = Some(callback);
    });
}

/// FLASH interrupt handler
pub fn irq_handler() {
    let mut address_tmp = 0u32;

    // Check FLASH operation error flags
    #[cfg(feature = "flash-bank2")]
    let has_error = FLASH.sr().read().wrprterr() ||
                    FLASH.sr().read().pgerr() ||
                    FLASH.sr2().read().wrprterr() ||
                    FLASH.sr2().read().pgerr();

    #[cfg(not(feature = "flash-bank2"))]
    let has_error = FLASH.sr().read().wrprterr() || FLASH.sr().read().pgerr();

    if has_error {
        critical_section::with(|cs| {
            let mut process = FLASH_PROCESS.borrow_ref_mut(cs);

            // Return the faulty address
            address_tmp = process.address;
            // Reset address
            process.address = 0xFFFFFFFF;

            // Save the Error code
            process.error_code = if FLASH.sr().read().wrprterr() {
                Some(HalError::WriteProtection)
            } else {
                Some(HalError::Programming)
            };

            // Stop the procedure ongoing
            process.procedure = FlashProcedure::None;
        });

        // FLASH error interrupt user callback
        critical_section::with(|cs| {
            if let Some(callback) = *ERROR_CALLBACK.borrow_ref(cs) {
                callback(address_tmp);
            }
        });
    }

    // Check FLASH End of Operation flag
    #[cfg(feature = "flash-bank2")]
    let eop = FLASH.sr().read().eop() || FLASH.sr2().read().eop();
    #[cfg(not(feature = "flash-bank2"))]
    let eop = FLASH.sr().read().eop();

    if eop {
        // Clear FLASH End of Operation pending bit
        #[cfg(feature = "flash-bank2")]
        {
            if FLASH.sr().read().eop() {
                FLASH.sr().modify(|w| w.set_eop(false));
            }
            if FLASH.sr2().read().eop() {
                FLASH.sr2().modify(|w| w.set_eop(false));
            }
        }
        #[cfg(not(feature = "flash-bank2"))]
        FLASH.sr().modify(|w| w.set_eop(false));

        critical_section::with(|cs| {
            let mut process = FLASH_PROCESS.borrow_ref_mut(cs);

            if process.procedure != FlashProcedure::None {
                match process.procedure {
                    FlashProcedure::ProgramHalfWord |
                    FlashProcedure::ProgramWord |
                    FlashProcedure::ProgramDoubleWord => {
                        // Decrement data remaining
                        process.data_remaining -= 1;

                        if process.data_remaining > 0 {
                            // Increment address
                            process.address += 2;
                            address_tmp = process.address;

                            // Shift to next 16-bit data
                            process.data >>= 16;

                            // Clear PG bit
                            #[cfg(feature = "flash-bank2")]
                            if address_tmp <= FLASH_BANK1_END {
                                FLASH.cr().modify(|w| w.set_pg(false));
                            } else {
                                FLASH.cr2().modify(|w| w.set_pg(false));
                            }
                            #[cfg(not(feature = "flash-bank2"))]
                            FLASH.cr().modify(|w| w.set_pg(false));

                            // Program next halfword
                            program_halfword(address_tmp, process.data as u16);
                        } else {
                            // Programming ended
                            let callback_addr = match process.procedure {
                                FlashProcedure::ProgramHalfWord => process.address,
                                FlashProcedure::ProgramWord => process.address - 2,
                                FlashProcedure::ProgramDoubleWord => process.address - 6,
                                _ => process.address,
                            };

                            // Reset process
                            process.address = 0xFFFFFFFF;
                            process.procedure = FlashProcedure::None;

                            // Call end of operation callback
                            if let Some(callback) = *END_OF_OP_CALLBACK.borrow_ref(cs) {
                                callback(callback_addr);
                            }
                        }
                    },
                    FlashProcedure::PageErase |
                    FlashProcedure::MassErase => {
                        // Add handling for these cases
                        process.procedure = FlashProcedure::None;
                        // Call appropriate callbacks
                    },
                    FlashProcedure::None => {
                        // Maybe log unexpected state
                    }
                }
            }
        });
    }

    // Disable interrupts if procedure is complete
    critical_section::with(|cs| {
        let process = FLASH_PROCESS.borrow_ref(cs);
        if process.procedure == FlashProcedure::None {
            #[cfg(feature = "flash-bank2")]
            {
                FLASH.cr().modify(|w| {
                    w.set_eopie(false);
                    w.set_errie(false);
                });
                FLASH.cr2().modify(|w| {
                    w.set_eopie(false);
                    w.set_errie(false);
                });
            }
            #[cfg(not(feature = "flash-bank2"))]
            FLASH.cr().modify(|w| {
                w.set_eopie(false);
                w.set_errie(false);
            });
        }
    });
}

/// Program a half-word (16-bit) at a specified address
///
/// # Safety
///
/// Caller must ensure:
/// - Address is properly aligned for 16-bit access
/// - Address points to valid flash memory region
/// - Flash is unlocked and properly configured for programming
unsafe fn program_halfword(address: u32, data: u16) {
    // Clean the error context
    // Note: In Rust we don't need a global error state since we use Result

    #[cfg(feature = "flash-bank2")]
    if address <= FLASH_BANK1_END {
        // Proceed to program the new data
        FLASH.cr().modify(|w| w.set_pg(true));
    } else {
        FLASH.cr2().modify(|w| w.set_pg(true));
    }
    #[cfg(not(feature = "flash-bank2"))]
    FLASH.cr().modify(|w| w.set_pg(true));

    // Write data to the address
    unsafe {
        core::ptr::write_volatile(address as *mut u16, data);
    }
}

/// Wait for a FLASH operation to complete
fn wait_for_last_operation(timeout: u32) -> Result<(), HalError> {
    let start = embassy_time::Instant::now();

    // Wait for the FLASH operation to complete by polling on BUSY flag
    while FLASH.sr().read().bsy() {
        if timeout != u32::MAX &&
           (embassy_time::Instant::now() - start).as_millis() > timeout as u64 {
            return Err(HalError::Timeout);
        }
    }

    // Check FLASH End of Operation flag
    if FLASH.sr().read().eop() {
        FLASH.sr().modify(|w| w.set_eop(false));
    }

    if FLASH.sr().read().wrprterr() ||
       FLASH.obr().read().opterr() ||
       FLASH.sr().read().pgerr() {

        // Clear error flags
        FLASH.sr().modify(|w| {
            w.set_wrprterr(false);
            w.set_pgerr(false);
        });

        // Return appropriate error
        if FLASH.sr().read().wrprterr() {
            return Err(HalError::WriteProtection);
        } else if FLASH.sr().read().pgerr() {
            return Err(HalError::Programming);
        } else {
            return Err(HalError::OptionByte);
        }
    }

    Ok(())
}

/// Unlock the FLASH control register access
pub fn unlock() -> Result<(), HalError> {
    if FLASH.cr().read().lock() {
        // Authorize the FLASH Registers access
        FLASH.keyr().write_value(FLASH_KEY1);
        FLASH.keyr().write_value(FLASH_KEY2);

        // Verify Flash is unlocked
        if FLASH.cr().read().lock() {
            return Err(HalError::Programming);
        }
    }

    #[cfg(feature = "flash-bank2")]
    if FLASH.cr2().read().lock() {
        // Authorize the FLASH BANK2 Registers access
        FLASH.keyr2().write(|w| w.set_keyr(FLASH_KEY1));
        FLASH.keyr2().write(|w| w.set_keyr(FLASH_KEY2));

        // Verify Flash BANK2 is unlocked
        if FLASH.cr2().read().lock() {
            return Err(HalError::Programming);
        }
    }

    Ok(())
}

/// Lock the FLASH control register access
pub fn lock() -> Result<(), HalError> {
    // Set the LOCK Bit to lock the FLASH Registers access
    FLASH.cr().modify(|w| w.set_lock(true));

    #[cfg(feature = "flash-bank2")]
    FLASH.cr2().modify(|w| w.set_lock(true));

    Ok(())
}

/// Unlock the FLASH Option Control Registers access
pub fn ob_unlock() -> Result<(), HalError> {
    if !FLASH.cr().read().optwre() {
        // Authorizes the Option Byte register programming
        FLASH.optkeyr().write_value(FLASH_OPTKEY1);
        FLASH.optkeyr().write_value(FLASH_OPTKEY2);

        if !FLASH.cr().read().optwre() {
            return Err(HalError::Programming);
        }
    }

    Ok(())
}

/// Lock the FLASH Option Control Registers access
pub fn ob_lock() -> Result<(), HalError> {
    // Clear the OPTWRE Bit to lock the FLASH Option Byte Registers access
    FLASH.cr().modify(|w| w.set_optwre(false));

    Ok(())
}

/// Launch the option byte loading
pub fn ob_launch() {
    // Initiates a system reset request to launch the option byte loading
    cortex_m::peripheral::SCB::sys_reset();
}

/// Wait for last operation on bank 2 to complete
#[cfg(feature = "flash-bank2")]
fn wait_for_last_operation_bank2(timeout: u32) -> Result<(), HalError> {
    let start = embassy_time::Instant::now();

    while FLASH.sr2().read().bsy() {
        if timeout != u32::MAX &&
           (embassy_time::Instant::now() - start).as_millis() > timeout as u64 {
            return Err(HalError::Timeout);
        }
    }

    if FLASH.sr2().read().eop() {
        FLASH.sr2().modify(|w| w.set_eop(false));
    }

    if FLASH.sr2().read().wrprterr() || FLASH.sr2().read().pgerr() {
        // Clear error flags
        FLASH.sr2().modify(|w| {
            w.set_wrprterr(false);
            w.set_pgerr(false)
        });

        if FLASH.sr2().read().wrprterr() {
            return Err(HalError::WriteProtection);
        } else {
            return Err(HalError::Programming);
        }
    }

    Ok(())
}

/// Program flash with interrupts enabled
pub fn program_it(program_type: ProgramType, address: u32, data: u64) -> Result<(), HalError> {
    // Initialize the flash process
    let mut process = FlashProcess {
        procedure: match program_type {
            ProgramType::HalfWord => FlashProcedure::ProgramHalfWord,
            ProgramType::Word => FlashProcedure::ProgramWord,
            ProgramType::DoubleWord => FlashProcedure::ProgramDoubleWord,
        },
        address,
        data,
        data_remaining: match program_type {
            ProgramType::HalfWord => 1,
            ProgramType::Word => 2,
            ProgramType::DoubleWord => 4,
        },
        error_code: None,
    };

    #[cfg(feature = "flash-bank2")]
    if address <= FLASH_BANK1_END {
        // Enable End of FLASH Operation and Error source interrupts for Bank 1
        FLASH.cr().modify(|w| {
            w.set_eopie(true);
            w.set_errie(true)
        });
    } else {
        // Enable interrupts for Bank 2
        FLASH.cr2().modify(|w| {
            w.set_eopie(true);
            w.set_errie(true)
        });
    }

    #[cfg(not(feature = "flash-bank2"))]
    FLASH.cr().modify(|w| {
        w.set_eopie(true);
        w.set_errie(true)
    });

    // Program first halfword
    program_halfword(address, data as u16);

    Ok(())
}

#[cfg(feature = "flash-bank2")]
const FLASH_BANK1_END: u32 = 0x0807FFFF; // Adjust this value based on your specific MCU
