use crate::comm::{Address, Comm, BROADCAST_ADDRESS, MAX_PAYLOAD_SIZE};
use crate::{flash, version, Mode};
use defmt::info;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use embedded_io_async::Write as AsyncWrite;
use heapless::Vec;

// Protocol message types and constants
const MAX_PANEL_SLOTS: usize = 32;

#[derive(Debug, Clone, Copy)]
pub struct PanelInfo {
    pub id: Address,
    pub boot_count: u8,
    pub rssi_master: i8,
    pub rssi_panel: i8,
    pub pirs: u8,
    pub slot: u8,
}

#[derive(Debug)]
#[repr(C)]
pub struct SetColorSlot {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug)]
#[repr(C)]
pub struct MapPanelSlot {
    pub id: u8,
}

#[derive(Debug)]
pub enum Message<'a> {
    // Master -> Panel messages
    SetColors { slots: &'a [SetColorSlot] },
    MapPanels { slots: &'a [MapPanelSlot] },
    Ping,
    Reset,
    SetStatus { status: u8 },
    Test { payload_size: u8 },

    // Panel -> Master messages
    SetColorReply { pirs: u8 },
    MapPanelReply { slot: u8 },
    Enumerate { boot_count: u8, rssi: u8 },
}

impl<'a> Message<'a> {
    pub fn parse(data: &'a [u8]) -> Option<(Address, Message<'a>)> {
        if data.len() < 2 {
            return None;
        }

        let from = Address(data[0]);
        let cmd = data[1];

        let payload = &data[2..];
        let msg = match cmd {
            b'C' => {
                let slots = unsafe {
                    core::slice::from_raw_parts(
                        payload.as_ptr() as *const SetColorSlot,
                        payload.len() / core::mem::size_of::<SetColorSlot>(),
                    )
                };
                Message::SetColors { slots }
            }
            b'M' => {
                let slots = unsafe {
                    core::slice::from_raw_parts(
                        payload.as_ptr() as *const MapPanelSlot,
                        payload.len() / core::mem::size_of::<MapPanelSlot>(),
                    )
                };
                Message::MapPanels { slots }
            }
            b'P' => Message::Ping,
            b'R' => Message::Reset,
            b'S' => {
                if payload.is_empty() {
                    return None;
                }
                Message::SetStatus { status: payload[0] }
            }
            b'_' => Message::Test {
                payload_size: if payload.is_empty() { 0 } else { payload[0] },
            },
            b'c' => {
                if payload.len() < 1 {
                    return None;
                }
                Message::SetColorReply { pirs: payload[0] }
            }
            b'm' => {
                if payload.len() < 1 {
                    return None;
                }
                Message::MapPanelReply { slot: payload[0] }
            }
            b'I' => {
                if payload.len() < 2 {
                    return None;
                }
                Message::Enumerate {
                    boot_count: payload[0],
                    rssi: payload[1],
                }
            }
            _ => return None,
        };

        Some((from, msg))
    }

    pub fn serialize(&self, from: Address) -> Option<Vec<u8, MAX_PAYLOAD_SIZE>> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[
            from.value(),
            match self {
                Message::SetColors { .. } => b'C',
                Message::MapPanels { .. } => b'M',
                Message::Ping => b'P',
                Message::Reset => b'R',
                Message::SetStatus { .. } => b'S',
                Message::Test { .. } => b'_',
                Message::SetColorReply { .. } => b'c',
                Message::MapPanelReply { .. } => b'm',
                Message::Enumerate { .. } => b'I',
            },
        ])
        .ok()?;

        match self {
            Message::SetColors { slots } => {
                let bytes = unsafe {
                    core::slice::from_raw_parts(
                        slots.as_ptr() as *const u8,
                        slots.len() * core::mem::size_of::<SetColorSlot>(),
                    )
                };
                buf.extend_from_slice(bytes).ok()?;
            }
            Message::MapPanels { slots } => {
                let bytes = unsafe {
                    core::slice::from_raw_parts(
                        slots.as_ptr() as *const u8,
                        slots.len() * core::mem::size_of::<MapPanelSlot>(),
                    )
                };
                buf.extend_from_slice(bytes).ok()?;
            }
            Message::SetStatus { status } => {
                buf.push(*status).ok()?;
            }
            Message::Test { payload_size } => {
                buf.push(*payload_size).ok()?;
            }
            Message::SetColorReply { pirs } => {
                buf.push(*pirs).ok()?;
            }
            Message::MapPanelReply { slot } => {
                buf.push(*slot).ok()?;
            }
            Message::Enumerate { boot_count, rssi } => {
                buf.extend_from_slice(&[*boot_count, *rssi]).ok()?;
            }
            _ => {}
        }

        Some(buf)
    }
}

pub struct Master<'a> {
    my_address: Address,
    comm: &'a mut Comm<'a>,
    panels: Vec<PanelInfo, MAX_PANEL_SLOTS>,
}

impl<'comm> Master<'comm> {
    pub fn new(my_address: Address, comm: &'comm mut Comm<'comm>) -> Self {
        Self {
            my_address,
            comm,
            panels: Vec::new(),
        }
    }

    async fn broadcast(&mut self, msg: Message<'_>) {
        if let Some(data) = msg.serialize(self.my_address) {
            self.comm.send_to(BROADCAST_ADDRESS, &data);
        }
    }

    async fn await_replies(&mut self, timeout_ms: u64) {
        let start = embassy_time::Instant::now();
        while start.elapsed() < Duration::from_millis(timeout_ms) {
            todo!()
            // if let Some(data) = self.comm.recv() {
            //     let rssi_master = 0; // TODO self.comm.last_rssi();
            //     if let Some((from, msg)) = Message::parse(data) {
            //         Self::handle_reply(rssi_master, &mut self.panels, from, msg).await;
            //     }
            // }
        }
    }

    async fn handle_reply(
        rssi_master: i8,
        panels: &mut Vec<PanelInfo, MAX_PANEL_SLOTS>,
        from: Address,
        msg: Message<'_>,
    ) {
        match msg {
            Message::Enumerate { boot_count, rssi } => {
                if let Some(panel) = panels.iter_mut().find(|p| p.id == from) {
                    panel.boot_count = boot_count;
                    panel.rssi_master = rssi_master;
                    panel.rssi_panel = rssi as i8;
                } else if panels.len() < MAX_PANEL_SLOTS {
                    panels
                        .push(PanelInfo {
                            id: from,
                            boot_count,
                            rssi_master,
                            rssi_panel: rssi as i8,
                            pirs: 0,
                            slot: 0,
                        })
                        .ok();
                }
            }
            Message::SetColorReply { pirs } => {
                if pirs != 0 {
                    if let Some(panel) = panels.iter_mut().find(|p| p.id == from) {
                        panel.pirs = pirs;
                    }
                }
            }
            Message::MapPanelReply { slot } => {
                if let Some(panel) = panels.iter_mut().find(|p| p.id == from) {
                    panel.slot = slot;
                }
            }
            _ => {}
        }
    }

    pub async fn handle_command(&mut self, command: &[u8], response: &mut impl AsyncWrite) {
        if command.is_empty() {
            response.write_all(b"?").await.unwrap();
        } else {
            let (command, args) = command.split_first().unwrap();
            match command {
                b'D' => self.set_default_mode(args, response).await,
                b'E' => self.enumerate(args, response).await,
                b'L' => self.set_colors(args, response).await,
                b'M' => self.map_panels(args, response).await,
                b'R' => self.reset_all(args, response).await,
                b'V' => response
                    .write_all(version::VERSION.as_bytes())
                    .await
                    .unwrap(),
                b'_' => self.test_message(args, response).await,
                _ => response.write_all(b"?").await.unwrap(),
            }
        }
        response.write_all(b"\r\n").await.unwrap();
    }

    async fn set_default_mode(&mut self, args: &[u8], response: &mut impl AsyncWrite) {
        info!("set_default_mode {}", core::str::from_utf8(args).unwrap());
        let mode = match args.first() {
            Some(b'M') => Mode::Master,
            Some(b'P') => Mode::Panel,
            _ => return response.write_all(b"?").await.unwrap(),
        };
        flash::set_default_mode(mode);
        response.write_all(b"OK").await.unwrap();
        Timer::after(Duration::from_millis(100)).await;
        cortex_m::peripheral::SCB::sys_reset();
    }

    async fn enumerate(&mut self, _args: &[u8], response: &mut impl AsyncWrite) {
        info!("enumerate");
        self.panels.clear();

        // Send ping message multiple times to ensure delivery
        for _ in 0..4 {
            self.broadcast(Message::Ping).await;
            self.await_replies(500).await;
        }

        todo!();
        // Format response as JSON array
        // let mut w = heapless::String::<256>::new();
        // write!(w, "[").unwrap();
        // for (i, panel) in self.panels.iter().enumerate() {
        //     if i > 0 {
        //         write!(w, ", ").unwrap();
        //     }
        //     write!(
        //         w,
        //         "{{\"id\":{}, \"bootCount\":{}, \"rssiM\":{}, \"rssiP\":{}}}",
        //         panel.id.value(),
        //         panel.boot_count,
        //         panel.rssi_master,
        //         panel.rssi_panel
        //     )
        //     .unwrap();
        // }
        // write!(w, "]").unwrap();

        // response.write_all(w.as_bytes()).await.unwrap();
    }

    async fn set_colors(&mut self, args: &[u8], response: &mut impl AsyncWrite) {
        info!("set_colors");
        if args.len() % 6 != 0 {
            return response.write_all(b"?").await.unwrap();
        }

        let mut slots = Vec::<SetColorSlot, MAX_PANEL_SLOTS>::new();
        let mut i = 0;
        while i < args.len() {
            let r = u8::from_str_radix(core::str::from_utf8(&args[i..i + 2]).unwrap(), 16).unwrap();
            let g =
                u8::from_str_radix(core::str::from_utf8(&args[i + 2..i + 4]).unwrap(), 16).unwrap();
            let b =
                u8::from_str_radix(core::str::from_utf8(&args[i + 4..i + 6]).unwrap(), 16).unwrap();
            slots.push(SetColorSlot { r, g, b }).ok();
            i += 6;
        }

        self.panels.clear();
        self.broadcast(Message::SetColors { slots: &slots }).await;
        self.await_replies(500).await;

        todo!();
        // Format response as JSON array of panel IDs that replied
        // let mut w = heapless::String::<256>::new();
        // write!(w, "[").unwrap();
        // for (i, panel) in self.panels.iter().enumerate() {
        //     if i > 0 {
        //         write!(w, ",").unwrap();
        //     }
        //     write!(w, "{}", panel.id.value()).unwrap();
        // }
        // write!(w, "]").unwrap();

        // response.write_all(w.as_bytes()).await.unwrap();
    }

    async fn map_panels(&mut self, args: &[u8], response: &mut impl AsyncWrite) {
        info!("map_panels");
        todo!();
        if args.len() % 2 != 0 || args.len() > MAX_PANEL_SLOTS * 2 {
            return response.write_all(b"?").await.unwrap();
        }

        let mut slots = Vec::<MapPanelSlot, MAX_PANEL_SLOTS>::new();
        let mut i = 0;
        while i < args.len() {
            let id =
                u8::from_str_radix(core::str::from_utf8(&args[i..i + 2]).unwrap(), 16).unwrap();
            slots.push(MapPanelSlot { id }).ok();
            i += 2;
        }

        let mut assigned = heapless::Vec::<bool, MAX_PANEL_SLOTS>::new();
        for _ in 0..slots.len() {
            assigned.push(false).ok();
        }

        let start = embassy_time::Instant::now();
        while !assigned.iter().all(|&x| x) && start.elapsed() < Duration::from_millis(5000) {
            self.panels.clear();
            self.broadcast(Message::MapPanels { slots: &slots }).await;
            self.await_replies(500).await;

            for panel in &self.panels {
                if panel.slot < assigned.len() as u8 {
                    assigned[panel.slot as usize] = true;
                }
            }

            Timer::after(Duration::from_millis(50)).await;
        }

        if assigned.iter().all(|&x| x) {
            response.write_all(b"OK assigned").await.unwrap();
        } else {
            response.write_all(b"PARTIAL").await.unwrap();
            for (i, &assigned) in assigned.iter().enumerate() {
                if assigned {
                    todo!();
                    // write!(response, " {}", i).unwrap();
                }
            }
        }
    }

    async fn reset_all(&mut self, _args: &[u8], response: &mut impl AsyncWrite) {
        info!("reset_all");
        todo!();
        // Get current boot counts
        let mut boot_counts = heapless::Vec::<(Address, u8), MAX_PANEL_SLOTS>::new();
        self.panels.clear();
        self.broadcast(Message::Ping).await;
        self.await_replies(500).await;
        for panel in &self.panels {
            boot_counts.push((panel.id, panel.boot_count)).ok();
        }

        // Send reset command until all panels have new boot counts
        for _ in 0..10 {
            self.broadcast(Message::Reset).await;
            Timer::after(Duration::from_millis(200)).await;

            self.panels.clear();
            self.broadcast(Message::Ping).await;
            self.await_replies(500).await;

            let mut all_reset = true;
            for panel in &self.panels {
                if let Some(old_count) = boot_counts.iter().find(|(id, _)| *id == panel.id) {
                    if old_count.1 == panel.boot_count {
                        all_reset = false;
                        break;
                    }
                }
            }

            if all_reset {
                response.write_all(b"OK").await.unwrap();
                return;
            }
        }

        response.write_all(b"FAILED").await.unwrap();
        for (i, panel) in self.panels.iter().enumerate() {
            if i > 0 {
                response.write_all(b",").await.unwrap();
            }
            // write!(response, "{}", panel.id.value()).unwrap();
        }
    }

    async fn test_message(&mut self, args: &[u8], response: &mut impl AsyncWrite) {
        info!("test_message");
        let payload_size = if args.is_empty() {
            0
        } else {
            core::str::from_utf8(args).unwrap().parse().unwrap_or(0)
        };
        self.broadcast(Message::Test { payload_size }).await;
        response.write_all(b"OK").await.unwrap();
    }

    pub async fn run<'b>(
        &mut self,
        command_signal: Signal<ThreadModeRawMutex, Vec<u8, 256>>,
        response_signal: Signal<ThreadModeRawMutex, Vec<u8, 256>>,
    ) {
        let mut response = WriteableVec(Vec::<u8, 256>::new());
        loop {
            let msg = command_signal.wait().await;
            self.handle_command(msg.as_slice(), &mut response).await;
            response_signal.signal(response.0.clone());
        }
    }
}

struct WriteableVec<const N: usize>(Vec<u8, N>);

impl<const N: usize> embedded_io_async::Write for WriteableVec<N> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        match self.0.extend_from_slice(buf) {
            Ok(()) => Ok(buf.len()),
            Err(_) => Err(Self::Error::Overflow),
        }
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, defmt::Format)]
enum WriteableVecError {
    Overflow,
}

impl embedded_io::Error for WriteableVecError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

impl<const N: usize> embedded_io::ErrorType for WriteableVec<N> {
    type Error = WriteableVecError;
}
