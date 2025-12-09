pub mod bytes_or_vecu8 {
    use crate::reusable_pool::global::VecU8;
    use crate::Bytes;

    #[cfg(feature = "std")]
    pub type Typ = Bytes;
    #[cfg(not(feature = "std"))]
    pub type Typ = VecU8;

    pub fn copy_from_slice(bytes: &[u8]) -> Typ {
        #[cfg(feature = "std")]
        {
            Bytes::copy_from_slice(bytes)
        }
        #[cfg(not(feature = "std"))]
        {
            VecU8::try_from_slice(bytes).expect("enough cap")
        }
    }

    pub fn new() -> Typ {
        #[cfg(feature = "std")]
        {
            Bytes::new()
        }
        #[cfg(not(feature = "std"))]
        {
            VecU8::default_for_reuse()
        }
    }
}

pub mod vec_u8_or_vecu8 {
    #[cfg(not(feature = "std"))]
    use crate::reusable_pool::global::VecU8;

    #[cfg(feature = "std")]
    pub type Typ = Vec<u8>;
    #[cfg(not(feature = "std"))]
    pub type Typ = VecU8;

    pub fn with_capacity(cap: usize) -> Typ {
        #[cfg(feature = "std")]
        {
            Vec::with_capacity(cap)
        }
        #[cfg(not(feature = "std"))]
        {
            VecU8::try_with_capacity(cap).expect("enough cap")
        }
    }

    pub fn copy_from_slice(bytes: &[u8]) -> Typ {
        #[cfg(feature = "std")]
        {
            Vec::from(bytes)
        }
        #[cfg(not(feature = "std"))]
        {
            VecU8::try_from_slice(bytes).expect("enough cap")
        }
    }

    pub fn new() -> Typ {
        #[cfg(feature = "std")]
        {
            Vec::new()
        }
        #[cfg(not(feature = "std"))]
        {
            VecU8::default_for_reuse()
        }
    }
}
