use core::ops::Range;
use defmt::debug;
use crate::fixed_vec::FixedVec;

pub struct LineBreaker {
    buffer: FixedVec<u8>,
    used_prefix: usize,
    discard: bool,
}

impl LineBreaker {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: FixedVec::new(capacity),
            used_prefix: 0,
            discard: false,
        }
    }

    /// Keep calling process() with chunks of input. It returns None if it needs
    /// more, or Some(line) if it found a line. The newline character is not
    /// included in the returned line.
    ///
    /// Works best if buf is at least 2*MAX_PACKET_SIZE. Otherwise it may drop
    /// the line after an over-long line.
    ///
    pub fn process(&mut self, buf: &[u8]) -> Option<&[u8]> {
        if self.used_prefix > 0 {
            let len = self.buffer.len();
            self.buffer.copy_within(self.used_prefix..len, 0);
            assert!(self.buffer.resize(len - self.used_prefix, 0).is_ok());
            self.used_prefix = 0;
        }

        if buf.is_empty() {
            return None;
        }

        let mut split = buf.splitn(2, |b| *b == b'\n');
        // We know buf is not empty, so unwrap is safe
        let first = split.next().unwrap();
        let rest = split.next();

        if let Some(rest) = rest {
            // Found a line ending
            if self.discard {
                // Discard the (partial) current line
                self.buffer.clear();
                // Save the beginning of the next line
                assert!(
                    self.buffer.extend_from_slice(rest).is_ok(),
                    "No room for line fragment"
                );
                self.discard = false;
                return None;
            }

            // Save the end of the current line
            if self.buffer.extend_from_slice(first).is_ok() {
                let line_len = self.buffer.len();
                if self.buffer.extend_from_slice(rest).is_ok() {
                    // We saved the beginning of the next line, yay happy path!
                    self.used_prefix = line_len;
                    return Some(&self.buffer[..line_len - 1]);
                }
                // We didn't have room for the beginning of the next line, so
                // discard the rest of it.
                self.discard = true;
                self.used_prefix = line_len;
                Some(&self.buffer[..line_len - 1])
            } else {
                // Line too long, discard it
                self.buffer.clear();
                self.discard = true;
                None
            }
        } else {
            // No line ending found, so just append the buffer
            if self.buffer.extend_from_slice(first).is_ok() {
                return None;
            }
            // Line too long, discard it
            self.buffer.clear();
            self.discard = true;
            None
        }
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.used_prefix = 0;
        self.discard = false;
    }
}
