use smallvec::SmallVec;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
#[repr(C)]
pub struct EncodingContext {
    pub hdr_size: u32,
    pub hdr_ptr: u32,
    pub data_ptr: u32,

    pub base_offset_stack: SmallVec<[u32; 8]>,
    pub current_offset_stack: SmallVec<[u32; 8]>,
}

impl Default for EncodingContext {
    fn default() -> Self {
        Self {
            hdr_size: 0,
            hdr_ptr: 0,
            data_ptr: 0,
            base_offset_stack: SmallVec::new(),
            current_offset_stack: SmallVec::new(),
        }
    }
}

impl EncodingContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_section(&mut self, base: u32) {
        self.base_offset_stack.push(base);
        self.current_offset_stack.push(base);
    }

    pub fn pop_section(&mut self) {
        self.base_offset_stack.pop();
        self.current_offset_stack.pop();
    }

    pub fn base_offset(&self) -> u32 {
        *self.base_offset_stack.last().unwrap_or(&32)
    }

    pub fn current_offset(&self) -> u32 {
        *self.current_offset_stack.last().unwrap_or(&32)
    }

    pub fn advance_current_offset(&mut self, size: u32) {
        if let Some(last) = self.current_offset_stack.last_mut() {
            *last += size;
        }
    }
}
