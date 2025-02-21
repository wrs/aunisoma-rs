use crate::board::{self, watchdog_petter, LedStrip, Pirs};
use crate::boot::get_boot_count;
use crate::comm::{BROADCAST_ADDRESS, Packet, PanelComm};
use crate::status_leds::StatusLEDs;
use crate::version;
use crate::{Interactor, Mode, comm::Address, flash::set_default_mode};
use core::fmt::Write;
use defmt::{debug, info, trace};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;
use num_enum::{IntoPrimitive, TryFromPrimitive};

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

#[derive(Debug, Clone, Copy, IntoPrimitive, TryFromPrimitive)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum Message {
    Ping = b'P',
    SetColor = b'C',
    MapPanels = b'M',
    Reset = b'R',
    SetStatus = b'S',
    Test = b'_',
    PingReply = b'I',
    SetColorReply = b'c',
    MapPanelsReply = b'm',
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

pub struct CmdProcessor<'a> {
    mode: Mode,
    interactor: Interactor<'a>,
    comm: PanelComm,
    address: Address,
    led_strip: LedStrip,
    pirs: Pirs,
    panels: heapless::Vec<PanelInfo, MAX_PANEL_SLOTS>,
    my_slot: Option<u8>,
    reply_buf: heapless::String<256>,
}

impl<'a> CmdProcessor<'a> {
    pub fn new(
        interactor: Interactor<'a>,
        comm: PanelComm,
        address: Address,
        led_strip: LedStrip,
        pirs: Pirs,
    ) -> Self {
        Self {
            mode: Mode::Master,
            interactor,
            comm,
            address,
            led_strip,
            pirs,
            panels: heapless::Vec::new(),
            my_slot: None,
            reply_buf: heapless::String::<256>::new(),
        }
    }

    pub async fn run_master(mut self) {
        self.mode = Mode::Master;
        info!("Master mode");
        loop {
            let mut buf = [0; 256];
            let line = self.interactor.read_command(&mut buf).await;
            // defmt::debug!("Command: {:a}", line);
            self.reply_buf.clear();
            self.handle_command(Mode::Master, line).await;
            self.interactor.reply(&self.reply_buf).await;
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
                    self.interactor.reply(&self.reply_buf).await;
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
            match select(
                self.comm.recv_packet(),
                Timer::after(Duration::from_millis(100)),
            )
            .await
            {
                Either::First(packet) => {
                    debug!("Received packet: {:?}", packet);
                }
                Either::Second(_) => {
                    board::pet_the_watchdog();
                }
            }
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
                let _ = self.reply_buf.push_str("ERROR Unknown command");
            }
        }
    }

    fn command_default_mode(&mut self, args: &[u8]) {
        if args.len() != 1 {
            let _ = self.reply_buf.push_str("ERROR Expected M, P, or S");
            return;
        }

        let new_mode = match args[0] {
            b'M' => Mode::Master,
            b'P' => Mode::Panel,
            b'S' => Mode::Spy,
            _ => {
                let _ = self.reply_buf.push_str("ERROR Expected M, P, or S");
                return;
            }
        };

        set_default_mode(new_mode);
        cortex_m::peripheral::SCB::sys_reset();
    }

    fn command_version(&mut self, _args: &[u8]) {
        let mode_str = match self.mode {
            Mode::Master => "Master",
            Mode::Panel => "Panel",
            Mode::Spy => "Spy",
        };

        let mut response = heapless::String::<128>::new();
        write!(
            response,
            "Aunisoma version {} ID={} Mode={} Comm={}",
            version::VERSION,
            self.address.value(),
            mode_str,
            self.comm.mode_name(),
        )
        .unwrap();

        let _ = self.reply_buf.push_str(response.as_str());
    }

    async fn command_enumerate(&mut self, _args: &[u8]) {
        let packet = Packet::new(self.address, BROADCAST_ADDRESS, Message::Ping);
        self.panels.clear();

        self.send_message(&packet, Duration::from_millis(40)).await;

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
        let _ = self.reply_buf.push_str(w.as_str());
    }

    async fn command_set_color(&mut self, args: &[u8]) {
        debug!("Set color: {:a}", args);
        // Each color takes 6 hex digits (2 each for R,G,B)
        if args.len() % 6 != 0 {
            let _ = self
                .reply_buf
                .push_str("ERROR Expected 6 hex digits per color");
            return;
        }

        let num_slots = args.len() / 6;
        if num_slots > MAX_PANEL_SLOTS {
            let _ = self.reply_buf.push_str("ERROR Too many slots");
            return;
        }

        let mut packet = Packet::new(self.address, BROADCAST_ADDRESS, Message::SetColor);

        // Parse RGB values for each slot
        for offset in (0..args.len()).step_by(2) {
            let b = match parse_hex_byte(&args[offset..offset + 2]) {
                Some(v) => v,
                None => {
                    let _ = self.reply_buf.push_str("ERROR Invalid hex byte");
                    return;
                }
            };

            packet.push_data(&[b]);
        }

        self.panels.clear();
        self.send_message(&packet, Duration::from_millis(MAX_PANEL_SLOTS as u64))
            .await;

        for slot in 0..num_slots {
            let pirs = match self.panels.iter().find(|p| p.slot as usize == slot) {
                Some(p) => p.pirs,
                None => 0,
            };
            let _ = self.reply_buf.push((b'0' + pirs) as char);
        }
    }

    async fn command_map_panels(&mut self, args: &[u8]) {
        // Each panel ID is 2 hex digits
        if args.len() % 2 != 0 || args.len() > MAX_PANEL_SLOTS * 2 {
            let _ = self.reply_buf.push_str("ERROR");
            return;
        }

        let mut packet = Packet::new(self.address, BROADCAST_ADDRESS, Message::MapPanels);

        let num_panels = args.len() / 2;

        let mut slot_ids = Vec::<u8, 32>::new();
        for i in 0..num_panels {
            let offset = i * 2;
            let id = match parse_hex_byte(&args[offset..offset + 2]) {
                Some(v) => v,
                None => {
                    let _ = self.reply_buf.push_str("ERROR Invalid hex byte");
                    return;
                }
            };
            slot_ids.push(id).unwrap();
        }

        packet.push_data(&slot_ids);

        let mut confirmed_slots: u32 = 0;

        let start = Instant::now();
        let timeout = Duration::from_millis(5000);

        // Send the packet multiple times to ensure all panels receive it
        for _ in 0..4 {
            self.panels.clear();
            self.send_message(&packet, Duration::from_millis(300)).await;

            // Check which slots were assigned
            for panel in self.panels.iter() {
                for (j, &id) in slot_ids.iter().enumerate() {
                    if panel.id.value() == id {
                        confirmed_slots |= 1 << j;
                        break;
                    }
                }
            }

            // Check if all slots are assigned
            let requested_mask = (1 << num_panels) - 1;
            if (confirmed_slots & requested_mask) == requested_mask {
                let _ = self.reply_buf.push_str("OK");
                return;
            }

            if start.elapsed() > timeout {
                break;
            }

            Timer::after(Duration::from_millis(50)).await;
        }

        let _ = self.reply_buf.push_str("FAILED ");
        for (i, &id) in slot_ids.iter().enumerate() {
            if (confirmed_slots & (1 << i)) == 0 {
                write!(&mut self.reply_buf, "{:02x}", id).unwrap();
            }
        }
    }

    async fn command_reset(&mut self, _args: &[u8]) {
        todo!()
    }

    async fn command_test_message(&mut self, args: &[u8]) {
        if args.len() != 2 {
            let _ = self.reply_buf.push_str("ERROR");
            return;
        }

        let len = match parse_hex_byte(&args[0..2]) {
            Some(v) => v,
            None => {
                let _ = self.reply_buf.push_str("ERROR Invalid hex byte");
                return;
            }
        };
        let mut packet = Packet::new(self.address, BROADCAST_ADDRESS, Message::Test);
        for i in 0..len {
            packet.push_data(&[i + 1]);
        }
        self.send_message(&packet, Duration::from_millis(10)).await;
        let _ = self.reply_buf.push_str("OK");
    }

    async fn send_message(&mut self, packet: &Packet, reply_time: Duration) {
        self.comm.send_packet(packet).await;

        let reply_deadline = Instant::now() + reply_time;

        loop {
            match select3(
                watchdog_petter(),
                self.comm.recv_packet(),
                Timer::at(reply_deadline),
            )
            .await
            {
                Either3::First(_) => {
                    // Watchdog petted
                }
                Either3::Second(packet) => {
                    self.handle_reply(packet);
                }
                Either3::Third(_) => {
                    break;
                }
            }
        }
    }

    fn handle_reply(&mut self, packet: Packet) {
        debug!("Received reply: {:?}", packet);

        let index = self.find_panel_index(packet.from);
        let panel = self.panels.get_mut(index).unwrap();

        match packet.tag {
            Message::PingReply => {
                if packet.data.len() == 2 {
                    panel.boot_count = packet.data[0];
                    panel.rssi_master = packet.data[1] as i8;
                } else {
                    debug!("PingReply: Invalid data length");
                }
            }
            Message::SetColorReply => {
                if packet.data.len() == 1 {
                    panel.pirs = packet.data[0];
                } else {
                    debug!("SetColorReply: Invalid data length");
                }
            }
            Message::MapPanelsReply => {
                if packet.data.len() == 1 {
                    panel.slot = packet.data[0];
                } else {
                    debug!("MapPanelsReply: Invalid data length");
                }
            }
            _ => {
                debug!(
                    "Unknown reply from {:x}: {:a}",
                    packet.from.0, packet.tag as u8 as char
                );
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

    // Incoming messages (panel mode)

    async fn handle_message(&mut self, packet: Packet) {
        let arrival_time = Instant::now();

        debug!("Received: {:?}", packet);

        let mut reply = Packet::new(self.address, packet.from, Message::Test);
        let reply_delay = Duration::from_millis(2);

        match packet.tag {
            Message::MapPanels => {
                self.handle_map_panels(&packet, &mut reply);
            }
            Message::Ping => {
                reply.tag = Message::PingReply;
                reply.push_data(&[get_boot_count()]);
                reply.push_data(&[0u8]);
            }
            Message::SetColor => {
                self.handle_set_color(&packet, &mut reply);
            }
            Message::SetStatus => {
                debug!("Set status");
                if packet.data.len() == 1 {
                    StatusLEDs::set_all(packet.data[0]);
                }
            }
            Message::Reset => {
                debug!("Reset");
                cortex_m::peripheral::SCB::sys_reset();
            }
            Message::Test => {
                debug!("Test message");
                reply.tag = Message::Test;
                let _ = reply.data.extend_from_slice(&packet.data);
            }
            _ => {
                debug!(
                    "Unknown message from {:x}: {:a}",
                    packet.from.0, packet.tag as u8
                );
                return;
            }
        }

        trace!(
            "Arrival {:?}us, reply {:?}us",
            arrival_time.as_micros(),
            Instant::now().as_micros()
        );

        Timer::at(arrival_time + reply_delay).await;
        self.comm.send_packet(&reply).await;
    }

    fn handle_map_panels(&mut self, packet: &Packet, reply: &mut Packet) {
        let num_slots = packet.data.len();
        if num_slots > MAX_PANEL_SLOTS {
            debug!("MapPanels: Too many slots");
            return;
        }

        if let Some((slot, _)) = packet
            .data
            .iter()
            .enumerate()
            .find(|&(_, &id)| id == self.address.value())
        {
            debug!("MapPanels: Mapping to slot {}", slot);
            self.my_slot = Some(slot as u8);
            reply.push_data(&[slot as u8]);
            reply.tag = Message::MapPanelsReply;
        } else {
            debug!("MapPanels: Didn't find my ID");
            self.my_slot = None;
        }
    }

    fn handle_set_color(&mut self, packet: &Packet, reply: &mut Packet) {
        if let Some(my_slot) = self.my_slot {
            if (my_slot + 1) as usize * 3 > packet.data.len() {
                debug!("SetColor: Not enough data");
                return;
            }

            let r = packet.data[my_slot as usize * 3];
            let g = packet.data[my_slot as usize * 3 + 1];
            let b = packet.data[my_slot as usize * 3 + 2];

            self.led_strip.set_colors(r, g, b);

            debug!("SetColor: RGB {:02x},{:02x},{:02x}", r, g, b);

            let pirs = (self.pirs.pir_1.is_high() as u8) | ((self.pirs.pir_2.is_high() as u8) << 1);

            reply.push_data(&[pirs]);
            reply.tag = Message::SetColorReply;
        } else {
            debug!("SetColor: Not mapped");
        }
    }
}

/// Parse two hex digits into a byte. Returns None if the input is not a valid
/// hex byte.
fn parse_hex_byte(input: &[u8]) -> Option<u8> {
    u8::from_str_radix(core::str::from_utf8(input).ok()?, 16).ok()
}
