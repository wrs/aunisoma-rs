#![no_std]
#![no_main]

extern crate alloc;

use comm::Address;
use command_port::CommandPort;
use defmt::{Format, info};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_time::Timer;
use embedded_alloc::LlffHeap as Heap;
use num_enum::TryFromPrimitive;
use status_leds::StatusLEDs;
use usb_port::UsbPort;

#[global_allocator]
static HEAP: Heap = Heap::empty();

// NOTE: Using Executor requires debugging with connect-under-reset.
// See "wfe interfering with RTT and flashing"
// https://github.com/embassy-rs/embassy/issues/1742

#[derive(Copy, Clone, Debug, PartialEq, Eq, Format, TryFromPrimitive)]
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

    spawner.must_spawn(blinker::blinker_task());

    let address = Address(0);
    let mode = boot::determine_mode(address);
    if board::controls().user_btn_is_pressed() {
        boot::toggle_mode(mode).await;
    }

    info!(
        "Aunisoma version {} ID={} Mode={}",
        version::VERSION,
        address.0,
        mode as u8
    );

    let cmd_port = CommandPort::new(board.cmd_port);
    let usb_port = UsbPort::new(board.usb, &spawner).await;

    let mut interactor = Interactor::new(cmd_port, usb_port);

    loop {
        let mut buf = [0; 256];
        let line = interactor.read_command(&mut buf).await;
        info!("Command: {:a}", line);
        interactor.reply(b"OK").await;
    }

    match mode {
        Mode::Master => {
            // master.run(serial).await;
        }
        Mode::Panel => {
            // let mut panel = Panel::new(app);
            // panel.run(serial, &mut comm).await;
        }
        Mode::Spy => loop {
            Timer::after_millis(100).await;
        },
    }
}

enum CommandSource {
    Serial,
    Usb,
}

struct Interactor<'a> {
    port: CommandPort<'a>,
    usb: UsbPort,
    source: CommandSource,
}

impl<'a> Interactor<'a> {
    fn new(port: CommandPort<'a>, usb: UsbPort) -> Self {
        Self {
            port,
            usb,
            source: CommandSource::Serial,
        }
    }

    async fn read_command<'b, const MAX_LEN: usize>(
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

    async fn reply(&mut self, line: &[u8]) {
        match self.source {
            CommandSource::Serial => self.port.write_line(line).await,
            CommandSource::Usb => self.usb.write_line(line).await,
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
mod comm;
mod command_port;
mod flash;
mod line_breaker;
mod status_leds;
mod usb_port;
mod version;
