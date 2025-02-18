#![no_std]
#![no_main]

extern crate alloc;

use cmd_processor::CmdProcessor;
use comm::{Address, CommMode, PanelComm, PanelRadio, PanelSerial};
use command_serial::CommandSerial;
use defmt::{Format, info};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embedded_alloc::LlffHeap as Heap;
use flash::get_my_id;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use status_leds::StatusLEDs;
use usb_port::UsbPort;

#[global_allocator]
static HEAP: Heap = Heap::empty();

// NOTE: Using Executor requires debugging with connect-under-reset.
// See "wfe interfering with RTT and flashing"
// https://github.com/embassy-rs/embassy/issues/1742

#[derive(Copy, Clone, Debug, PartialEq, Eq, Format, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum Mode {
    Master = 1,
    Panel = 2,
    Spy = 3,
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    defmt::info!("\n-----\nMain task started\n-----");

    // Initialize the heap
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 4096;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        #[allow(static_mut_refs)]
        unsafe {
            HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE)
        };
    }

    boot::check_boot_status();

    let board = board::hookup();

    StatusLEDs::init(board.status_leds);

    // spawner.must_spawn(blinker::blinker_task());

    let address = Address(get_my_id());

    let mode = boot::determine_mode(address);
    if board::controls().user_btn_is_pressed() {
        boot::toggle_mode(mode).await;
    }

    info!(
        "Aunisoma version {} ID={} Mode={:?}",
        version::VERSION,
        address.0,
        mode
    );

    let cmd_port = CommandSerial::new(board.cmd_port);
    let usb_port = UsbPort::new(board.usb, address, &spawner).await;
    let interactor = Interactor::new(cmd_port, usb_port);

    let radio = PanelRadio::new(board.radio);
    let panel_serial = PanelSerial::new(board.panel_bus, address);
    let comm = PanelComm::new(CommMode::Serial, address, radio, panel_serial);

    let cmd_processor = CmdProcessor::new(interactor, comm, address, board.led_strip, board.pirs);
    match mode {
        Mode::Master => cmd_processor.run_master().await,
        Mode::Panel => cmd_processor.run_panel().await,
        Mode::Spy => cmd_processor.run_spy().await,
    }
}

enum CommandSource {
    Serial,
    Usb,
}

/// Interactor reads commands from the serial port and USB port, and replies to
/// the port that sent the command.
///
pub struct Interactor<'a> {
    port: CommandSerial<'a>,
    usb: UsbPort,
    source: CommandSource,
}

impl<'a> Interactor<'a> {
    fn new(port: CommandSerial<'a>, usb: UsbPort) -> Self {
        Self {
            port,
            usb,
            source: CommandSource::Serial,
        }
    }

    pub async fn read_command<'b, const MAX_LEN: usize>(
        &mut self,
        buf: &'b mut [u8; MAX_LEN],
    ) -> &'b [u8] {
        let mut cmd_buf = [0; MAX_LEN];
        let mut usb_buf = [0; MAX_LEN];
        let cmd_line = self.port.read_line(&mut cmd_buf);
        let usb_line = self.usb.read_line(&mut usb_buf);

        let line = match select(cmd_line, usb_line).await {
            Either::First(line) => {
                self.source = CommandSource::Serial;
                line
            }
            Either::Second(line) => {
                self.source = CommandSource::Usb;
                line
            }
        };

        buf[..line.len()].copy_from_slice(line);
        &buf[..line.len()]
    }

    pub async fn reply(&mut self, line: &str) {
        match self.source {
            CommandSource::Serial => self.port.write_line(line.as_bytes()).await,
            CommandSource::Usb => self.usb.write_line(line.as_bytes()).await,
        }
    }
}

#[inline(never)]
#[panic_handler]
fn core_panic(info: &core::panic::PanicInfo<'_>) -> ! {
    defmt::error!("Panic: {:?}", info);
    loop {}
}

mod blinker;
mod board;
mod boot;
mod cmd_processor;
mod comm;
mod command_serial;
mod fixed_vec;
mod flash;
mod line_breaker;
mod status_leds;
mod usb_port;
mod version;
