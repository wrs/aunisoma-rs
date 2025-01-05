#![allow(dead_code)]

use core::fmt::Write;
use defmt::info;
use embassy_stm32::gpio::Input;
use embassy_time::{Duration, Timer};
use heapless::Vec;

use crate::get_boot_count;
use crate::comm::{Address, MAX_PAYLOAD_SIZE};

pub const MAX_PANEL_SLOTS: usize = 32;

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

    pub fn serialize(&self, from: Address) -> Option<Vec<u8, { MAX_PAYLOAD_SIZE }>> {
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

pub struct Panel {
    my_address: Address,
    my_slot: Option<u8>,
    pir1: Input<'static>,
    pir2: Input<'static>,
}

impl Panel {
    pub fn new(my_address: Address, pir1: Input<'static>, pir2: Input<'static>) -> Self {
        Self {
            my_address,
            my_slot: None,
            pir1,
            pir2,
        }
    }

    pub fn get_pirs(&self) -> u8 {
        ((self.pir1.is_high() as u8) << 0) | ((self.pir2.is_high() as u8) << 1)
    }

    pub async fn handle_message(
        &mut self,
        from: Address,
        msg: Message<'_>,
    ) -> Option<Message<'static>> {
        match msg {
            Message::SetColors { slots } => {
                if let Some(slot) = self.my_slot {
                    if (slot as usize) < slots.len() {
                        let slot = &slots[slot as usize];
                        // TODO: Set LED colors
                        info!("Set colors r={} g={} b={}", slot.r, slot.g, slot.b);

                        let pirs = self.get_pirs();
                        if pirs != 0 {
                            return Some(Message::SetColorReply { pirs });
                        }
                    }
                }
            }
            Message::MapPanels { slots } => {
                for (i, slot) in slots.iter().enumerate() {
                    if Address(slot.id) == self.my_address {
                        self.my_slot = Some(i as u8);
                        return Some(Message::MapPanelReply { slot: i as u8 });
                    }
                }
                self.my_slot = None;
            }
            Message::Ping => {
                return Some(Message::Enumerate {
                    boot_count: get_boot_count(),
                    rssi: 0, // TODO: Get RSSI from radio
                });
            }
            Message::Reset => {
                Timer::after(Duration::from_millis(100)).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
            Message::SetStatus { status } => {
                // TODO: Set status LEDs
                info!("Set status {}", status);
            }
            _ => {}
        }
        None
    }
}
