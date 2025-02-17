use crate::boot::get_boot_count;
use crate::comm::{BROADCAST_ADDRESS, MAX_PAYLOAD_SIZE, Packet, PanelComm};
use crate::fixed_vec::FixedVec;
use crate::version;
use crate::{Interactor, Mode, comm::Address, flash::set_default_mode};
use core::error::Error;
use core::fmt::Write;
use defmt::{debug, info};
use embassy_futures::select::{self, Either, select};
use embassy_futures::yield_now;
use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;
use num_enum::TryFromPrimitive;

// Protocol message types and constants
const MAX_PANEL_SLOTS: usize = 32;

/*
    M protocol lines

    Master and panel mode commands

    | Command                   | Response                                              | Description                                                                  |
    | ------------------------- | ----------------------------------------------------- | ---------------------------------------------------------------------------- |
    | Default Mode<br>`D`{mode} | `OK` or an error message                              | Sets the default mode. {mode} is `M` for master, `P` for panel, `S` for spy. |
    | Version<br>`V`            | Build version string<br>E.g., `"4fa9105"`             | Firmware version. Can be used as a safe way to synchronize the protocol.     |
    Master-only commands

    | Command                        | Response                                                                                                                                                                                                 | Description                                                                                                                                                                                                                     |
    | ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
    | Enumerate<br>`E`               | JSON `[{id, bootCount, rssiM, rssiP}]`<br>E.g., `[{"id":12, "bootCount": 123, "rssiM":-35, "rssiP":-42]}, {"id":9, bootCount: 97, "rssiM":-35, "rssiP":-42}]`                                            | Enumerates the IDs and signal strength of the reachable panels. `bootCount` is an arbitrary number that changes on each reboot. `rssiM` is the RSSI on the master, `rssiP` is the RSSI on the panel.                            |
    | Set Color<br>`L`\[{r}{g}{b}\]* | *Single* digits for PIR values from panels, in map order. PIR1 is 1, PIR2 is 2, both is 3.<br>E.g., after `M04080a` and `L<18 digits>`, if panel 8 has PIR1 and panel 10 has PIR1&2, responds `013`.<br> | Sets the panel colors. The order of the panels must have been set previously by the `M` command. Colors are RGB as two hex digits each. E.g., `L818283717273` sets the first two mapped panels to colors 0x818283 and 0x717273. |
    | Map Panels<br>`M` \[{id}\]*    | `OK` or `FAILED 010203`                                                                                                                                                                                  | Sets the panel IDs for the Set Color command Panel IDs are two ASCII hex bytes. E.g., `M04080a` sets the panel order to 4, 8, 10.                                                                                               |
    | Reset All<br>`R`               | `OK` or `FAILED 010203`                                                                                                                                                                                  | Restarts all controllers.                                                                                                                                                                                                       |

    P Protocol messages

    | Command                            | Reply                | Description                                                                                                           |
    | ---------------------------------- | -------------------- | --------------------------------------------------------------------------------------------------------------------- |
    | Ping<br>`P`                        | `I`{bootCount}{rssi} | {rssi} is a signed byte of RSSI                                                                                       |
    | Set Color<br>`C`\[{r}{g}{b}\]*     | `c`{PIR}             | {r}, {g}, {b} are RGB intensity bytes.<br>{PIR} byte: bitwise OR of 1 for PIR1, 2 for PIR2 |    | Map Panels<br>`M`[{id}]*           | `m`{slot}            | Sets the ID to slot mapping to be used when interpreting Set Color commands                                           |
    | Reset<br>`R`                       | *none*               | Restart the controller                                                                                                |
    | Set Status<br>`S`{status}          | *none*               | Sets the status lights on the controller to the low four bits of the byte {s}                                         |

*/

#[derive(Debug, Clone, Copy, TryFromPrimitive)]
#[repr(u8)]
pub enum Command {
    DefaultMode = b'D',
    Version = b'V',
    Enumerate = b'E',
    SetColor = b'L',
    MapPanels = b'M',
    Reset = b'R',
    TestMessage = b'_',
}

#[derive(Debug, Clone, Copy, TryFromPrimitive)]
#[repr(u8)]
pub enum Message {
    Ping = b'P',
    SetColor = b'C',
    SetStatus = b'S',
    Reset = b'R',
    Test = b'_',
    PingReply = b'I',
    SetColorReply = b'c',
    MapPanelReply = b'm',
}

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
pub struct SetColorSlot {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug)]
pub struct MapPanelSlot {
    pub id: u8,
}

pub struct CmdProcessor<'a> {
    mode: Mode,
    interactor: Interactor<'a>,
    comm: PanelComm,
    address: Address,
    panels: FixedVec<PanelInfo>,
    mapping: FixedVec<u8>,
    reply_buf: FixedVec<u8>,
}

impl<'a> CmdProcessor<'a> {
    pub fn new(interactor: Interactor<'a>, comm: PanelComm, address: Address) -> Self {
        Self {
            mode: Mode::Master,
            interactor,
            comm,
            address,
            panels: FixedVec::new(MAX_PANEL_SLOTS),
            mapping: FixedVec::new(MAX_PANEL_SLOTS),
            reply_buf: FixedVec::new(256),
        }
    }

    pub async fn run_master(mut self) {
        self.mode = Mode::Master;
        info!("Master mode");
        loop {
            let mut buf = [0; 256];
            let line = self.interactor.read_command(&mut buf).await;
            self.reply_buf.clear();
            self.handle_command(Mode::Master, line).await;
            if !self.reply_buf.is_empty() {
                self.interactor.reply(&self.reply_buf).await;
            }
        }
    }

    pub async fn run_panel(mut self) {
        self.mode = Mode::Panel;
        info!("Panel mode");
        loop {
            let mut cmd_buf = [0; 256];
            match select(
                self.interactor.read_command(&mut cmd_buf),
                self.comm.recv_packet(),
            )
            .await
            {
                Either::First(line) => {
                    self.reply_buf.clear();
                    self.handle_command(Mode::Panel, line).await;
                    if !self.reply_buf.is_empty() {
                        self.interactor.reply(&self.reply_buf).await;
                    }
                }
                Either::Second(packet) => {
                    self.handle_message(packet).await;
                }
            }
        }
    }

    pub async fn run_spy(mut self) {
        self.mode = Mode::Spy;
        info!("Spy mode");
        loop {
            yield_now().await;
        }
    }

    async fn handle_command(&mut self, mode: Mode, line: &[u8]) {
        if line.is_empty() {
            return;
        }

        self.reply_buf.clear();

        // Try to parse the first byte as a Command
        let cmd_byte = line[0];
        let args = &line[1..];

        match Command::try_from(cmd_byte) {
            Ok(Command::DefaultMode) => self.command_default_mode(args),
            Ok(Command::Version) => self.command_version(args),

            Ok(Command::Enumerate) if mode == Mode::Master => self.command_enumerate(args).await,
            Ok(Command::SetColor) if mode == Mode::Master => self.command_set_color(args).await,
            Ok(Command::MapPanels) if mode == Mode::Master => self.command_map_panels(args).await,
            Ok(Command::Reset) if mode == Mode::Master => self.command_reset(args).await,
            Ok(Command::TestMessage) if mode == Mode::Master => {
                self.command_test_message(args).await
            }

            _ => {
                let _ = self.reply_buf.extend_from_slice(b"ERROR Unknown command");
            }
        }
    }

    fn command_default_mode(&mut self, args: &[u8]) {
        if args.len() != 1 {
            let _ = self
                .reply_buf
                .extend_from_slice(b"ERROR Expected M, P, or S");
            return;
        }

        let new_mode = match args[0] {
            b'M' => Mode::Master,
            b'P' => Mode::Panel,
            b'S' => Mode::Spy,
            _ => {
                let _ = self
                    .reply_buf
                    .extend_from_slice(b"ERROR Expected M, P, or S");
                return;
            }
        };

        set_default_mode(new_mode);
        cortex_m::peripheral::SCB::sys_reset();
    }

    fn command_version(&mut self, _args: &[u8]) {
        let mode_str = match self.mode {
            Mode::Master => "MASTER",
            Mode::Panel => "PANEL",
            Mode::Spy => "SPY",
        };

        let mut response = heapless::String::<128>::new();
        write!(
            response,
            "Aunisoma version {} ID={} Mode={}={} Comm=?",
            version::VERSION,
            self.address.value(),
            self.mode as u8,
            mode_str,
        )
        .unwrap();

        let _ = self.reply_buf.extend_from_slice(response.as_bytes());
    }

    async fn command_enumerate(&mut self, _args: &[u8]) {
        let packet = Packet::new(self.address, BROADCAST_ADDRESS, Message::Ping as u8);
        self.panels.clear();

        self.send_message(packet, Duration::from_millis(40)).await;

        // Format response as JSON array
        let mut w = heapless::String::<256>::new();
        write!(w, "[").unwrap();
        for (i, panel) in self.panels.iter().enumerate() {
            if i > 0 {
                write!(w, ", ").unwrap();
            }
            write!(
                w,
                "{{\"id\":{}, \"bootCount\":{}, \"rssiM\":{}, \"rssiP\":{}}}",
                panel.id.value(),
                panel.boot_count,
                panel.rssi_master,
                panel.rssi_panel
            )
            .unwrap();
        }
        write!(w, "]").unwrap();
        let _ = self.reply_buf.extend_from_slice(w.as_bytes());
    }

    async fn command_set_color(&mut self, args: &[u8]) {
        // Each color takes 6 hex digits (2 each for R,G,B)
        if args.len() % 6 != 0 {
            let _ = self
                .reply_buf
                .extend_from_slice(b"ERROR Expected 6 hex digits per color");
            return;
        }

        let num_colors = args.len() / 6;
        if num_colors > MAX_PANEL_SLOTS {
            let _ = self.reply_buf.extend_from_slice(b"ERROR Too many slots");
            return;
        }

        let mut packet = Packet::new(self.address, BROADCAST_ADDRESS, Message::SetColor as u8);

        // Parse RGB values for each slot
        for offset in (0..args.len()).step_by(2) {
            let b = match parse_hex_byte(&args[offset..offset + 2]) {
                Some(v) => v,
                None => {
                    let _ = self.reply_buf.extend_from_slice(b"ERROR Invalid hex byte");
                    return;
                }
            };

            packet.push_data(&[b]);
        }

        self.panels.clear();
        self.send_message(packet, Duration::from_millis(10)).await;

        for panel in self.panels.iter() {
            let _ = self.reply_buf.push(b'0' + panel.pirs);
        }
    }

    async fn send_message(&mut self, packet: Packet, reply_time: Duration) {
        self.comm.send_packet(packet).await;

        let start = Instant::now();
        let deadline = start + reply_time;
        loop {
            let timeout = Timer::at(deadline);

            match select(self.comm.recv_packet(), timeout).await {
                Either::First(packet) => {
                    self.handle_reply(packet);
                }
                Either::Second(_) => {
                    debug!("Timeout at {:?}", Instant::now() - start);
                    break;
                }
            }
        }
    }

    fn handle_reply(&mut self, packet: Packet) {
        debug!("Received reply: {:?}", packet);

        let command = match Message::try_from(packet.tag) {
            Ok(command) => command,
            Err(_) => {
                debug!("Unknown tag: {:?}", packet.tag);
                return;
            }
        };

        let index = self.find_panel_index(packet.from);
        let panel = self.panels.get_mut(index).unwrap();

        match command {
            Message::PingReply => {
                panel.boot_count = packet.data[0];
                panel.rssi_master = packet.data[1] as i8;
            }
            Message::SetColorReply => {
                panel.pirs = packet.data[0];
            }
            Message::MapPanelReply => {
                panel.slot = packet.data[0];
            }
            _ => {
                debug!("Unknown tag: {:?}", packet.tag);
            }
        }
    }

    fn find_panel_index(&mut self, id: Address) -> usize {
        if let Some(index) = self
            .panels
            .iter()
            .enumerate()
            .find(|(_, panel)| panel.id == id)
        {
            return index.0;
        }

        let panel = PanelInfo {
            id,
            boot_count: 0,
            rssi_master: 0,
            rssi_panel: 0,
            pirs: 0,
            slot: 0,
        };
        self.panels.push(panel).unwrap();
        self.panels.len() - 1
    }

    async fn command_map_panels(&mut self, args: &[u8]) {
        // Each panel ID is 2 hex digits
        if args.len() % 2 != 0 || args.len() > MAX_PANEL_SLOTS * 2 {
            self.reply_buf.extend_from_slice(b"ERROR").unwrap();
            return;
        }

        let mut slots: Vec<MapPanelSlot, MAX_PANEL_SLOTS> = Vec::new();
        let num_panels = args.len() / 2;

        // Parse panel IDs
        for i in 0..num_panels {
            let offset = i * 2;
            let id = match parse_hex_byte(&args[offset..offset + 2]) {
                Some(v) => v,
                None => {
                    self.reply_buf
                        .extend_from_slice(b"ERROR Invalid hex byte")
                        .unwrap();
                    return;
                }
            };
            slots.push(MapPanelSlot { id }).unwrap();
        }

        todo!("Process slots and generate response")
    }

    async fn command_reset(&mut self, _args: &[u8]) {
        todo!()
    }

    async fn command_test_message(&mut self, args: &[u8]) {
        if args.len() != 2 {
            self.reply_buf.extend_from_slice(b"ERROR").unwrap();
            return;
        }

        let len = match parse_hex_byte(&args[0..2]) {
            Some(v) => v,
            None => {
                self.reply_buf
                    .extend_from_slice(b"ERROR Invalid hex byte")
                    .unwrap();
                return;
            }
        };
        let mut packet = Packet::new(self.address, BROADCAST_ADDRESS, Message::Test as u8);
        for i in 0..len {
            packet.push_data(&[i + 1]);
        }
        self.send_message(packet, Duration::from_millis(10)).await;
        self.reply_buf.extend_from_slice(b"OK").unwrap();
    }

    async fn handle_message(&mut self, packet: Packet) {
        let tag = match Message::try_from(packet.tag) {
            Ok(tag) => tag,
            Err(_) => {
                debug!("Unknown reply from {:?}: {:a}", packet.from.0, packet.tag);
                return;
            }
        };

        debug!("Received: {:?}", packet);

        match tag {
            Message::Ping => {
                let mut reply = Packet::new(self.address, packet.from, Message::PingReply as u8);
                reply.push_data(&[get_boot_count()]);
                reply.push_data(&[0u8]);
                // TODO reply at correct time
                self.comm.send_packet(reply).await;
            }
            Message::SetColor => {
                debug!("Set color");
            }
            Message::SetStatus => {
                debug!("Set status");
            }
            Message::Reset => {
                debug!("Reset");
            }
            Message::Test => {
                debug!("Test message");
            }
            _ => {
                debug!("Unknown message from {:?}: {:a}", packet.from.0, packet.tag);
            }
        }
    }
}

/// Parse two hex digits into a byte. Returns None if the input is not a valid
/// hex byte.
fn parse_hex_byte(input: &[u8]) -> Option<u8> {
    if input.len() < 2 {
        return None;
    }

    let high = match input[0] {
        b'0'..=b'9' => input[0] - b'0',
        b'a'..=b'f' => input[0] - b'a' + 10,
        b'A'..=b'F' => input[0] - b'A' + 10,
        _ => return None,
    };

    let low = match input[1] {
        b'0'..=b'9' => input[1] - b'0',
        b'a'..=b'f' => input[1] - b'a' + 10,
        b'A'..=b'F' => input[1] - b'A' + 10,
        _ => return None,
    };

    Some((high << 4) | low)
}
