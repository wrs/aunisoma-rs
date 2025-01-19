#![no_std]
#![no_main]

use crate::blinker::blinker_task;
use comm::{Address, Comm, CommImpl, RxBuffer};
use core::{
    cell::{Cell, RefCell},
    panic::PanicInfo,
};
use cortex_m::singleton;
use debug_port::DebugPort;
use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::Input;
use embassy_sync::{
    blocking_mutex::{
        self,
        raw::{NoopRawMutex, ThreadModeRawMutex},
        CriticalSectionMutex,
    },
    mutex::Mutex,
    zerocopy_channel::{Channel, Receiver},
};
use embassy_time::{Instant, Timer};
use num_enum::TryFromPrimitive;
use panel::Panel;
use panel_bus::PanelBus;
use radio::{radio_receiver_task, Radio};
use serial::Serial;
// use panic_itm as _;
#[cfg(feature = "use-itm")]
use defmt_itm as _;
#[cfg(feature = "use-rtt")]
use defmt_rtt as _;
use status_leds::StatusLEDs;
// use panic_halt as _;

#[link_section = ".noinit"]
static mut BOOT_COUNT: u8 = 0;

static mut IS_WARM_BOOT: bool = false;
#[link_section = ".noinit"]
static mut BOOT_MAGIC: u32 = 0;
const BOOT_MAGIC_VALUE: u32 = 0xdeadbeef;

// NOTE: Using Executor requires debugging with connect-under-reset.
// See "wfe interfering with RTT and flashing"
// https://github.com/embassy-rs/embassy/issues/1742

static mut RX_BUFFER: [RxBuffer; 8] = [const { RxBuffer::new() }; 8];

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    check_boot_status();

    #[cfg(feature = "use-itm")]
    {
        let cp = cortex_m::Peripherals::take().unwrap();
        defmt_itm::enable(cp.ITM);
    }

    defmt::info!("Main task started");

    let board = board::hookup();

    StatusLEDs::init(board.status_leds);

    spawner.must_spawn(blinker_task());

    let mut comm = Comm {
        name: "PanelBus",
        receive_callback: None,
        address: Address(flash::get_my_id()),
        actual: None,
    };

    let panel_bus = PanelBus::new(
        comm.address,
        Some(&(App::receive_callback as fn())),
        board.panel_bus_usart,
        board.panel_bus_usart_tx,
        board.panel_bus_usart_tx_dma,
        board.panel_bus_usart_rx_dma,
        board.ser_out_en,
    )
    .await;

    let radio = Radio::new(
        comm.address,
        Some(&(App::receive_callback as fn())),
        board.rf_spi,
        board.rf_sck,
        board.rf_mosi,
        board.rf_miso,
        board.rf_cs,
    );
    let radio_mutex = singleton!(RADIO_MUTEX: Mutex<ThreadModeRawMutex, RefCell<Radio>> = Mutex::new(RefCell::new(radio))).unwrap();

    let rx_channel = singleton!(RX_CHANNEL: Channel<'static, ThreadModeRawMutex, RxBuffer> = Channel::new(unsafe { &mut RX_BUFFER })).unwrap();
    let (rx_sender, rx_receiver) = rx_channel.split();

    let mut using_radio = false;

    let radio = radio_mutex.lock().await;
    if radio.borrow_mut().init(board.rf_rst).await.is_ok() {
        using_radio = true;
        comm.actual = Some(CommImpl::Radio(radio_mutex));
        spawner.must_spawn(radio_receiver_task(
            radio_mutex,
            board.rf_int,
            board.rf_exti,
            rx_sender,
        ));
    } else {
        defmt::info!("No radio");
        comm.actual = Some(CommImpl::PanelBus(panel_bus));
    }

    let mut rx_buffer = [0; 128];
    let mut tx_buffer = [0; 128];
    let debug_port = DebugPort::<256>::new(
        board.dbg_usart,
        board.dbg_usart_rx,
        board.dbg_usart_tx,
        &mut rx_buffer,
        &mut tx_buffer,
    );

    let usb_serial = usb::init(
        spawner,
        board.usb,
        board.usb_pullup,
        board.usb_dp,
        board.usb_dm,
    )
    .await;

    {
        let mut app = App {
            mode: Mode::Panel,
            address: comm.address,
            using_radio,
            rx_receiver,
            user_btn: board.user_btn,
            pir_1: board.pir_1,
            pir_2: board.pir_2,
        };
        app.determine_mode().await;

        spawner.must_spawn(app_task(app, Serial::UsbSerial(usb_serial), comm));
    }

    loop {
        StatusLEDs::set(3);
        Timer::after_millis(100).await;
        StatusLEDs::reset(3);
        Timer::after_millis(100).await;
    }
}

fn check_boot_status() {
    // Safety: We just booted so there aren't any threads
    unsafe {
        BOOT_COUNT = BOOT_COUNT.wrapping_add(1);
        // Disallow zero so we can use it as a sentinel value
        if BOOT_COUNT == 0 {
            BOOT_COUNT = 1;
        }

        info!("BOOT_MAGIC={:x}", BOOT_MAGIC);
        if BOOT_MAGIC == BOOT_MAGIC_VALUE {
            IS_WARM_BOOT = true;
        } else {
            IS_WARM_BOOT = false;
            core::ptr::write_volatile(&raw mut BOOT_MAGIC, BOOT_MAGIC_VALUE);
        }

        info!("is_warm_boot={}", IS_WARM_BOOT);
    }
}

pub fn is_warm_boot() -> bool {
    // Safety: This is only written once at boot time.
    unsafe { IS_WARM_BOOT }
}

pub fn get_boot_count() -> u8 {
    // Safety: This is only written once at boot time.
    unsafe { BOOT_COUNT }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, TryFromPrimitive)]
#[repr(u8)]
pub enum Mode {
    Master = 1,
    Panel = 2,
    Spy = 3,
}

pub struct App {
    mode: Mode,
    address: Address,
    using_radio: bool,
    rx_receiver: Receiver<'static, ThreadModeRawMutex, RxBuffer>,
    user_btn: Input<'static>,
    pir_1: Input<'static>,
    pir_2: Input<'static>,
}

static RECEIVE_TIME: CriticalSectionMutex<Cell<Instant>> =
    CriticalSectionMutex::new(Cell::new(Instant::from_ticks(0)));

#[embassy_executor::task]
async fn app_task(app: App, serial: Serial<'static, 256>, mut comm: Comm<'static>) {
    info!(
        "Aunisoma version {} ID={} Mode={}",
        version::VERSION,
        app.address.0,
        app.mode as u8
    );

    match app.mode {
        Mode::Master => {
            // let mut master = Master::new(self.my_id, comm);
            // master.run(serial).await;
        }
        Mode::Panel => {
            let mut panel = Panel::new(app);
            panel.run(serial, &mut comm).await;
        }
        Mode::Spy => loop {
            Timer::after_millis(100).await;
        },
    }
}

impl App {
    fn get_pirs(&self) -> u8 {
        ((self.pir_1.is_high() as u8) << 0) | ((self.pir_2.is_high() as u8) << 1)
    }

    fn receive_callback() {
        RECEIVE_TIME.lock(|time| {
            time.set(Instant::now());
        });
    }

    fn user_btn_pressed(&self) -> bool {
        self.user_btn.is_high()
    }

    /// Board 0 is always in Spy mode.
    ///
    /// Boards store their default mode in flash. Uninitialized boards default to
    /// Panel mode. If the button is down at boot, the default mode will be
    /// switched between Master and Panel. The default mode can also be changed
    /// with the 'D' command.

    async fn determine_mode(&mut self) {
        let mut mode = flash::get_default_mode();

        if self.address == Address(0) {
            mode = Mode::Spy;
        } else {
            if self.user_btn_pressed() {
                mode = self.toggle_mode().await;
            }
        }

        match mode {
            Mode::Master => {
                StatusLEDs::set(1);
            }
            Mode::Panel => {
                StatusLEDs::set(2);
            }
            Mode::Spy => {
                StatusLEDs::set(1);
                StatusLEDs::set(2);
            }
        }

        self.mode = mode;
    }

    /// Toggle between Master and Panel modes
    ///
    async fn toggle_mode(&mut self) -> Mode {
        let new_mode = match self.mode {
            Mode::Master => Mode::Panel,
            Mode::Panel => Mode::Master,
            _ => self.mode,
        };

        flash::set_default_mode(new_mode);

        info!(
            "Mode is now {}",
            if new_mode == Mode::Master {
                "Master"
            } else {
                "Panel"
            }
        );

        // Blink lights until button is released

        while self.user_btn_pressed() {
            StatusLEDs::set_all(0xF);
            Timer::after_millis(250).await;
            StatusLEDs::set_all(0);
            Timer::after_millis(250).await;
        }

        Timer::after_millis(250).await;

        cortex_m::peripheral::SCB::sys_reset();
    }
}

#[inline(never)]
#[panic_handler] // built-in ("core") attribute
fn core_panic(info: &PanicInfo<'_>) -> ! {
    defmt::error!("Panic: {:?}", info);
    loop {}
}

mod blinker;
mod board;
mod comm;
mod debug_port;
mod flash;
mod line_breaker;
mod master;
mod panel;
mod panel_bus;
mod radio;
mod ring_buffer;
mod serial;
mod status_leds;
mod usb;
mod version;
