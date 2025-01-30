use core::cell::RefCell;

use embassy_sync::blocking_mutex::{raw::ThreadModeRawMutex, Mutex};

pub const MAX_PAYLOAD_SIZE: usize = 64;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Address(pub u8);

impl Address {
    pub fn value(&self) -> u8 {
        self.0
    }
}

pub const BROADCAST_ADDRESS: Address = Address(0xFF);

pub type RxBuffer = heapless::Vec<u8, MAX_PAYLOAD_SIZE>;

pub type ReceiveCallback = &'static fn();

pub enum CommImpl<'a> {
    Radio(&'static Mutex<ThreadModeRawMutex, RefCell<crate::radio::Radio>>),
    PanelBus(crate::panel_bus::PanelBus<'a>),
}

pub struct Comm<'a> {
    pub name: &'static str,
    pub receive_callback: Option<ReceiveCallback>,
    pub address: Address,
    pub actual: Option<CommImpl<'a>>,
}

pub enum CommError {
    Radio(crate::radio::RadioError),
    PanelBus(crate::panel_bus::PanelBusError),
}

impl From<crate::radio::RadioError> for CommError {
    fn from(e: crate::radio::RadioError) -> Self {
        CommError::Radio(e)
    }
}

impl From<crate::panel_bus::PanelBusError> for CommError {
    fn from(e: crate::panel_bus::PanelBusError) -> Self {
        CommError::PanelBus(e)
    }
}

impl<'a> Comm<'a> {
    pub async fn last_rssi(&self) -> i8 {
        match &self.actual {
            Some(CommImpl::Radio(radio)) => radio.lock(|radio| radio.borrow().last_rssi()),
            Some(CommImpl::PanelBus(_)) => 0,
            None => 0,
        }
    }

    pub async fn send_to(&mut self, to_addr: Address, data: &[u8]) -> Result<(), CommError> {
        match &mut self.actual {
            Some(CommImpl::Radio(radio)) => {
                radio.lock(|radio| radio.borrow_mut().send_to(to_addr, data)).map_err(CommError::Radio)
            }
            Some(CommImpl::PanelBus(panel_bus)) => Ok(panel_bus.send_to(to_addr, data)?),
            None => Err(CommError::Radio(crate::radio::RadioError::NoRadio)),
        }
    }
}
