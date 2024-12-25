use core::fmt::Error as FmtError;
use core::fmt::Write;
use cortex_m::peripheral::ITM;

#[inline(never)]
pub fn itm_write_str(s: &str) -> Result<(), FmtError> {
    let itm = ITM::PTR;
    for byte in s.bytes() {
        unsafe {
            // SAFETY: Writing to the ITM register is atomic so the worst that can happen is
            // interleaved output.
            (*itm).stim[0].write_u8(byte);
        }
    }
    Ok(())
}

pub struct GlobalITMWriter {}

impl Write for GlobalITMWriter {
    fn write_str(&mut self, s: &str) -> Result<(), FmtError> {
        itm_write_str(s)
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        // write!(logger::GlobalITMWriter {}, $($arg)*).ok();
        (logger::GlobalITMWriter {}).write_str($($arg)*).ok();
    }};
}
