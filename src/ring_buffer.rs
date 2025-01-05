use core::mem::MaybeUninit;

use cortex_m::peripheral::{scb::VectActive, SCB};
use embassy_time::{Duration, Instant};

pub struct RingBuffer<T, const N: usize>
where
    T: Default,
{
    buffer: MaybeUninit<[T; N]>,
    next_read_idx: usize,
    next_write_idx: usize,
}

impl<T, const N: usize> RingBuffer<T, N>
where
    T: Default,
{
    pub fn new() -> Self {
        Self {
            buffer: MaybeUninit::<[T; N]>::uninit(),
            next_read_idx: 0,
            next_write_idx: 0,
        }
    }

    pub fn next_read(&mut self) -> Option<&T> {
        if self.is_empty() {
            return None;
        }
        // Safety: The item has been initialized by the caller before write_done()
        let item = unsafe { &self.buffer.assume_init_ref()[self.next_read_idx] };
        self.next_read_idx = (self.next_read_idx + 1) % N;
        Some(item)
    }

    pub fn next_read_with_timeout(&mut self, timeout: Duration) -> Option<&T> {
        let start = Instant::now();
        while self.is_empty() {
            if start.elapsed() > timeout {
                return None;
            }
        }
        self.next_read()
    }

    pub fn next_write(&mut self) -> Option<*mut T> {
        if self.is_full() {
            if SCB::vect_active() != VectActive::ThreadMode {
                // Inside an interrupt, so we can't block
                SCB::sys_reset();
            }

            let start = Instant::now();
            while self.is_full() {
                if start.elapsed() > Duration::from_millis(1000) {
                    // We seem to be stuck, so reset the system
                    SCB::sys_reset();
                }
            }
        }

        let item = unsafe { &mut self.buffer.assume_init_mut()[self.next_write_idx] };
        self.next_write_idx = (self.next_write_idx + 1) % N;
        Some(item)
    }

    pub fn write_done(&mut self, item: *const T) {
        assert_eq!(
            item,
            unsafe { self.buffer.as_ptr().add(self.next_write_idx) } as *const T
        );
        self.next_write_idx = (self.next_write_idx + 1) % N;
    }

    pub fn is_empty(&self) -> bool {
        self.next_read_idx == self.next_write_idx
    }

    pub fn is_full(&self) -> bool {
        self.next_read_idx == (self.next_write_idx + 1) % N
    }

    pub fn len(&self) -> usize {
        if self.next_read_idx >= self.next_write_idx {
            self.next_read_idx - self.next_write_idx
        } else {
            N - self.next_write_idx + self.next_read_idx
        }
    }

    pub fn flush(&mut self) {
        self.next_read_idx = 0;
        self.next_write_idx = 0;
    }
}
