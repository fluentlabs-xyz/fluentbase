#[macro_export]
macro_rules! typ_align {
    ($typ:ty) => {
        core::mem::align_of::<$typ>()
    };
}

#[macro_export]
macro_rules! typ_name {
    ($typ:ty) => {
        core::any::type_name::<$typ>()
    };
}

#[macro_export]
macro_rules! typ_size {
    ($typ:ty) => {
        core::mem::size_of::<$typ>()
    };
}

#[macro_export]
macro_rules! println_typ_size {
    ($typ:ty) => {
        println!(
            "size_of::<{}>() = {}",
            $crate::typ_name!($typ),
            $crate::typ_size!($typ)
        )
    };
}

#[macro_export]
macro_rules! map_addr {
    ($is:ident, $mmh:expr, $addr:ident) => {
        if $is {
            $mmh.map_vm_addr_to_host(
                $addr as u64,
                $crate::word_size::common::FIXED_PTR_BYTE_SIZE as u64,
            )
            .unwrap()
        } else {
            $addr
        }
    };
    ($mmh:expr, $addr:ident) => {
        $crate::map_addr!($mmh, $addr, $crate::word_size::common::FIXED_PTR_BYTE_SIZE)
    };
    ($mmh:expr, $addr:ident, $len:expr) => {
        $mmh.map_vm_addr_to_host($addr as u64, $len as u64).unwrap()
    };
}
#[macro_export]
macro_rules! remap_addr {
    ($mmh:expr, $addr:ident) => {
        let $addr = $crate::map_addr!($mmh, $addr);
    };
    ($is:ident, $mmh:expr, $addr:ident) => {
        let $addr = if $is {
            $crate::map_addr!($mmh, $addr)
        } else {
            $addr
        };
    };
}
