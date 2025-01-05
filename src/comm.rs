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

pub type ReceiveCallback = fn();

pub trait Comm: Send {
    fn name(&self) -> &str;

    fn set_receive_callback(&mut self, callback: Option<ReceiveCallback>);
    fn set_address(&mut self, address: Address);
    fn set_default_to_rx_mode(&mut self);
    fn set_spy_mode(&mut self, is_spy_mode: bool);
    fn last_rssi(&self) -> i8;

    fn send_to(&mut self, to_addr: Address, data: &[u8]) -> bool;
    fn available(&self) -> bool;
    fn recv(&mut self) -> Option<&RxBuffer>;
}
