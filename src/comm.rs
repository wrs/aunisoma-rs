pub const MAX_PAYLOAD_SIZE: usize = 64;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Address(pub u8);

impl Address {
    pub fn value(&self) -> u8 {
        self.0
    }
}

pub const BROADCAST_ADDRESS: Address = Address(0xFF);
