use bytes::BufMut;

/// Count bytes without storing them
pub(crate) struct ByteCounter {
    count: usize,
}

impl ByteCounter {
    #[inline]
    pub fn new() -> Self {
        Self { count: 0 }
    }
    
    pub fn count(&self) -> usize {
        self.count
    }
}

unsafe impl BufMut for ByteCounter {
    #[inline]
    fn remaining_mut(&self) -> usize {
        usize::MAX
    }

    #[inline]
    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.count += cnt;
    }

    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        unreachable!(
            "ByteCounter does not support chunk_mut(). \
            This is a counting-only implementation. \
            All writes must use put_* methods."
        )
    }

    #[inline]
    fn put_slice(&mut self, src: &[u8]) {
        self.count += src.len();
    }

    #[inline]
    fn put_bytes(&mut self, _: u8, cnt: usize) {
        self.count += cnt;
    }

    // Override common methods for efficiency
    #[inline]
    fn put_u8(&mut self, _: u8) {
        self.count += 1;
    }

    #[inline]
    fn put_u32(&mut self, _: u32) {
        self.count += 4;
    }

    #[inline]
    fn put_u32_le(&mut self, _: u32) {
        self.count += 4;
    }
}
