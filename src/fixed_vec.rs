use alloc::vec::Vec;
use core::ops::Range;

pub struct FixedVec<T: Copy>(Vec<T>);

impl<T: Copy> FixedVec<T> {
    pub fn new(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn resize(&mut self, new_len: usize, value: T) -> Result<(), ()> {
        if new_len > self.0.capacity() {
            return Err(());
        }

        self.0.resize(new_len, value);
        Ok(())
    }

    pub fn copy_within(&mut self, src: Range<usize>, dst: usize) {
        self.0.copy_within(src, dst);
    }

    pub fn extend_from_slice(&mut self, src: &[T]) -> Result<(), ()> {
        if self.0.len() + src.len() > self.0.capacity() {
            return Err(());
        }

        self.0.extend_from_slice(src);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

impl<T: Copy> core::ops::Deref for FixedVec<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        self.0.as_slice()
    }
}
