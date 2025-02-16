use core::error::Error;

use crate::comm::PanelComm;
use crate::fixed_vec::FixedVec;
use crate::version;
use crate::{Interactor, Mode, comm::Address, flash::set_default_mode};
use defmt::info;
use embassy_futures::yield_now;
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
    | Enumerate<br>`E`               | JSON `[{id, bootCount, rssiM, rssiP}]`<br>E.g., `[{"id":12, “bootCount”: 123, "rssiM":-35, "rssiP":-42]}, {"id":9, bootCount: 97, "rssiM":-35, "rssiP":-42}]`                                            | Enumerates the IDs and signal strength of the reachable panels. `bootCount` is an arbitrary number that changes on each reboot. `rssiM` is the RSSI on the master, `rssiP` is the RSSI on the panel.                            |
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
}

#[derive(Debug)]
pub enum Message {
    // Master -> Panel messages
    SetColors {
        slots: [SetColorSlot; MAX_PANEL_SLOTS],
    },
    MapPanels {
        slots: [MapPanelSlot; MAX_PANEL_SLOTS],
    },
    Ping,
    Reset,
    SetStatus {
        status: u8,
    },
    Test {
        payload_size: u8,
    },

    // Panel -> Master messages
    SetColorReply {
        pirs: u8,
    },
    MapPanelReply {
        slot: u8,
    },
    PingReply {
        boot_count: u8,
        rssi: u8,
    },
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
    interactor: Interactor<'a>,
    comm: PanelComm,
    panels: FixedVec<PanelInfo>,
}

impl<'a> CmdProcessor<'a> {
    pub fn new(interactor: Interactor<'a>, comm: PanelComm) -> Self {
        Self {
            interactor,
            comm,
            panels: FixedVec::new(MAX_PANEL_SLOTS),
        }
    }

    pub async fn run(mut self, mode: Mode) {
        loop {
            let mut buf = [0; 256];
            let line = self.interactor.read_command(&mut buf).await;
            let mut reply_buf = [0; 256];
            let reply = handle_command(mode, &mut self.comm, line, &mut reply_buf).await;
            if !reply.is_empty() {
                self.interactor.reply(reply).await;
            }
        }
    }

    pub async fn run_spy(mut self) {
        todo!()
    }
}

async fn handle_command<'a>(
    mode: Mode,
    comm: &mut PanelComm,
    line: &[u8],
    reply_buf: &'a mut [u8],
) -> &'a [u8] {
    if line.is_empty() {
        return &[];
    }

    // Try to parse the first byte as a Command
    let cmd_byte = line[0];
    let cmd = match Command::try_from(cmd_byte) {
        Ok(cmd) => cmd,
        Err(_) => return b"?",
    };

    let args = &line[1..];

    match cmd {
        Command::DefaultMode => handle_default_mode(args, reply_buf),
        Command::Version => handle_version(args, reply_buf),
        Command::Enumerate => handle_enumerate(comm, args, reply_buf).await,
        Command::SetColor => handle_set_color(comm, args, reply_buf).await,
        Command::MapPanels => handle_map_panels(comm, args, reply_buf).await,
        Command::Reset => handle_reset(comm, args, reply_buf).await,
    }
}

/// Parse two hex digits into a byte. Returns None if the input is not a valid
/// hex byte.
///
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

fn handle_default_mode<'a>(args: &[u8], _reply_buf: &'a mut [u8]) -> &'a [u8] {
    if args.len() != 1 {
        return b"ERROR Expected M, P, or S";
    }

    let new_mode = match args[0] {
        b'M' => Mode::Master,
        b'P' => Mode::Panel,
        b'S' => Mode::Spy,
        _ => return b"ERROR Expected M, P, or S",
    };

    set_default_mode(new_mode);

    cortex_m::peripheral::SCB::sys_reset();
}

fn handle_version<'a>(_args: &[u8], reply_buf: &'a mut [u8]) -> &'a [u8] {
    let version = version::VERSION.as_bytes();
    reply_buf[..version.len()].copy_from_slice(version);
    &reply_buf[..version.len()]
}

async fn handle_enumerate<'a>(
    comm: &mut PanelComm,
    args: &[u8],
    reply_buf: &'a mut [u8],
) -> &'a [u8] {
    todo!()
}

async fn handle_set_color<'a>(
    comm: &mut PanelComm,
    args: &[u8],
    reply_buf: &'a mut [u8],
) -> &'a [u8] {
    // Each color takes 6 hex digits (2 each for R,G,B)
    if args.len() % 6 != 0 {
        return b"?";
    }

    let num_colors = args.len() / 6;
    if num_colors > MAX_PANEL_SLOTS {
        return b"?";
    }

    let mut packet: Vec<u8, { MAX_PANEL_SLOTS * 3 }> = Vec::new();
    packet.push(Command::SetColor as u8).unwrap();

    // Parse RGB values for each slot
    for offset in (0..args.len()).step_by(2) {
        let b = match parse_hex_byte(&args[offset..offset + 2]) {
            Some(v) => v,
            None => return b"?",
        };

        packet.push(b).unwrap();
    }

    let replies = send_command(comm, packet.as_slice()).await;

    b"OK"
}

async fn send_command(comm: &mut PanelComm, packet: &[u8]) -> Vec<u8, 256> {
    let mut replies = Vec::new();
    comm.send_packet(packet).await;
    comm.recv_packet(&mut replies).await;
    replies
}

async fn handle_map_panels<'a>(
    comm: &mut PanelComm,
    args: &[u8],
    reply_buf: &'a mut [u8],
) -> &'a [u8] {
    // Each panel ID is 2 hex digits
    if args.len() % 2 != 0 || args.len() > MAX_PANEL_SLOTS * 2 {
        return &[];
    }

    let mut slots: Vec<MapPanelSlot, MAX_PANEL_SLOTS> = Vec::new();
    let num_panels = args.len() / 2;

    // Parse panel IDs
    for i in 0..num_panels {
        let offset = i * 2;
        let id = match parse_hex_byte(&args[offset..offset + 2]) {
            Some(v) => v,
            None => return &[],
        };
        slots.push(MapPanelSlot { id }).unwrap();
    }

    todo!("Process slots and generate response")
}

async fn handle_reset<'a>(comm: &mut PanelComm, args: &[u8], reply_buf: &'a mut [u8]) -> &'a [u8] {
    todo!()
}

pub async fn run_spy<'a>(mut interactor: Interactor<'a>, comm: PanelComm) {
    info!("Spy mode");
    loop {
        yield_now().await;
    }
}
