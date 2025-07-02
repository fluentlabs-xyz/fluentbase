#[derive(Debug, PartialOrd, PartialEq, Clone)]
#[repr(C)]
#[derive(Default)]
pub struct EncodingContext {
    /// Start position (absolute offset) of the current container's header within the buffer.
    pub hdr_ptr: u32,
    /// Current absolute position in the buffer for writing the next tail data.
    pub data_ptr: u32,

    /// Indicates if the offset for the current dynamic field has already been written by an
    /// upper-level container.
    pub offset_written: bool,

    /// will be removed
    /// Total size of the current container's header section. Data starts immediately after this.
    pub hdr_size: u32,
    // depricated
    pub depth: u32,
}

impl EncodingContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_hs(hdr_size: u32) -> Self {
        Self {
            hdr_size,
            data_ptr: hdr_size,
            ..Self::default()
        }
    }
    // /// Creates a nested encoding context for a dynamic field within the current container.
    // ///
    // /// # Arguments
    // /// - `nested_hdr_size`: Size of the header section of the nested container.
    // ///
    // /// # Returns
    // /// A new `EncodingContext` correctly positioned for nested encoding.
    // pub fn nested(
    //     &self,
    //     nested_hdr_ptr: u32,
    //     nested_hdr_size: u32,
    //     offset_written: bool,
    // ) -> EncodingContext {
    //     EncodingContext {
    //         hdr_ptr: nested_hdr_ptr,
    //
    //         // Nested container's data immediately follows its header.
    //         data_ptr: nested_hdr_ptr + nested_hdr_size,
    //         // data_ptr: self.data_ptr + nested_hdr_size,
    //
    //         // Total header size of the nested container.
    //         hdr_size: nested_hdr_size,
    //
    //         offset_written,
    //
    //         // Compatibility fields (to be removed later)
    //         depth: self.depth + 1,
    //     }
    // }
    /// Creates a nested context specifically for struct fields (header_ptr remains the same).
    /// nested container - we need to update start and end positions and increase depth
    /// since offsets relative for current container - we need to
    pub fn nested_struct(&self, hdr_size: u32, offset_written: bool) -> EncodingContext {
        EncodingContext {
            hdr_ptr: 0,
            data_ptr: hdr_size,
            // is it a good way? maybe we can do better
            offset_written,
            depth: self.depth + 1,
            // we don't need it actually
            hdr_size,
        }
    }

    /// Creates a nested context specifically for vectors (header_ptr set to current data_ptr).
    pub fn nested_vector(&self, hdr_size: u32, offset_written: bool) -> EncodingContext {
        EncodingContext {
            hdr_ptr: self.data_ptr,
            data_ptr: self.data_ptr + hdr_size,
            hdr_size,
            offset_written,
            depth: self.depth + 1,
        }
    }
}
