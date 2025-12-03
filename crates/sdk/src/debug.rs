use core::fmt::{self, Write as _};

pub const DEBUG_LOG_MAXIMUM_LEN: usize = 1_000;

#[cfg(target_arch = "wasm32")]
struct SyncUnsafeCell<T>(core::cell::UnsafeCell<T>);

#[cfg(target_arch = "wasm32")]
impl<T> SyncUnsafeCell<T> {
    const fn new(v: T) -> Self {
        Self(core::cell::UnsafeCell::new(v))
    }
    #[inline]
    fn get(&self) -> *mut T {
        self.0.get()
    }
}

#[cfg(target_arch = "wasm32")]
unsafe impl<T> Sync for SyncUnsafeCell<T> {}

#[cfg(target_arch = "wasm32")]
static BUF: SyncUnsafeCell<[u8; DEBUG_LOG_MAXIMUM_LEN]> =
    SyncUnsafeCell::new([0; DEBUG_LOG_MAXIMUM_LEN]);

#[cfg(not(target_arch = "wasm32"))]
static BUF: std::sync::Mutex<[u8; DEBUG_LOG_MAXIMUM_LEN]> =
    std::sync::Mutex::new([0; DEBUG_LOG_MAXIMUM_LEN]);

struct SliceWriter<'a> {
    buf: &'a mut [u8],
    len: usize,
    truncated: bool,
}
impl<'a> SliceWriter<'a> {
    #[inline]
    fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            truncated: false,
        }
    }
    #[inline]
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
    #[inline]
    fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap()
    }
}

impl fmt::Write for SliceWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let space = self.buf.len().saturating_sub(self.len);
        if s.len() <= space {
            self.buf[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
            self.len += s.len();
        } else {
            let take = space;
            if take > 0 {
                self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
                self.len += take;
            }
            self.truncated = true;
        }
        Ok(())
    }
}

pub const fn two_component_start(path: &'static str) -> usize {
    let bytes = path.as_bytes();
    let mut i = bytes.len();
    let mut count = 0usize;
    while i > 0 {
        i -= 1;
        if bytes[i] == b'/' || bytes[i] == b'\\' {
            count += 1;
            if count == 2 {
                return i + 1;
            }
        }
    }
    0
}
#[inline]
pub fn short_file(path: &'static str) -> &'static str {
    &path[two_component_start(path)..]
}

/// Returns (start_index, end_index) of the function name inside `full`.
pub const fn fn_name_bounds(full: &'static str) -> (usize, usize) {
    let bytes = full.as_bytes();
    let mut len = bytes.len();

    // Remove trailing "::{{closure...}}" suffix if present
    // Find the substring start of "::{{closure"
    let mut i = 0usize;
    while i + 10 <= len {
        // "::{{closure" length = 10 chars
        // manual compare because const fn can't use `starts_with`
        let mut match_closure = true;
        let spec = b"::{{closure";
        let mut j = 0usize;
        while j < spec.len() {
            if bytes[i + j] != spec[j] {
                match_closure = false;
                break;
            }
            j += 1;
        }
        if match_closure {
            len = i; // cut suffix
            break;
        }
        i += 1;
    }

    // Now find last "::" *outside* generics <...>
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut k = len;
    while k > 1 {
        k -= 1;
        let b = bytes[k];
        if b == b'>' {
            depth += 1;
        } else if b == b'<' {
            if depth > 0 {
                depth -= 1;
            }
        } else if b == b':' && bytes[k - 1] == b':' && depth == 0 {
            start = k + 1; // after "::"
            break;
        }
    }

    // Strip trailing generics: find first '<' after start
    let mut end = len;
    let mut d2 = 0i32;
    let mut j2 = start;
    while j2 < len {
        let b = bytes[j2];
        if b == b'<' && d2 == 0 {
            end = j2;
            break;
        }
        if b == b'<' {
            d2 += 1;
        } else if b == b'>' && d2 > 0 {
            d2 -= 1;
        }
        j2 += 1;
    }

    (start, end)
}

/// Extract the caller function name from a type-name string like:
///   fluentbase_contracts_evm::main_entry<...>::{{closure}}
/// Returns: "main_entry"
#[inline]
pub fn extract_fn_name(full: &'static str) -> &'static str {
    let (start, end) = fn_name_bounds(full);
    &full[start..end]
}

pub fn debug_log_write_with_loc(
    file: &'static str,
    line: u32,
    func: &'static str,
    args: fmt::Arguments<'_>,
) -> Result<(), fmt::Error> {
    #[cfg(target_arch = "wasm32")]
    let w: &mut [u8] = unsafe { &mut *BUF.get() };
    #[cfg(not(target_arch = "wasm32"))]
    let mut w_guard = BUF.lock().unwrap();
    #[cfg(not(target_arch = "wasm32"))]
    let w = w_guard.as_mut_slice();

    let mut w = SliceWriter::new(w);

    // prefix
    write!(&mut w, "[{}:{} {}] ", short_file(file), line, func)?;
    w.write_fmt(args)?;

    // If truncated → append "..."
    if w.truncated {
        let remaining = w.buf.len().saturating_sub(w.len);
        if remaining >= 3 {
            // enough space → append
            w.buf[w.len..w.len + 3].copy_from_slice(b"...");
            w.len += 3;
        } else if w.buf.len() >= 3 {
            // not enough → overwrite last 3 bytes
            let end = w.buf.len();
            w.buf[end - 3..end].copy_from_slice(b"...");
            w.len = end;
        }
    }

    #[cfg(target_arch = "wasm32")]
    unsafe {
        #[link(wasm_import_module = "fluentbase_v1preview")]
        extern "C" {
            fn _debug_log(ptr: *const u8, len: u32);
        }
        let b = w.as_bytes();
        _debug_log(b.as_ptr(), b.len() as u32);
    }

    #[cfg(feature = "std")]
    {
        println!("{}", w.as_str());
    }

    Ok(())
}

#[cfg(debug_assertions)]
#[macro_export]
macro_rules! debug_log {
    () => {{
        let _ = $crate::debug::debug_log_write_with_loc(
            $crate::debug::short_file(file!()),
            line!(),
            $crate::debug::extract_fn_name(core::any::type_name_of_val(&|| {})),
            core::format_args!(""),
        );
    }};
    ($msg:expr) => {{
        let _ = $crate::debug::debug_log_write_with_loc(
            $crate::debug::short_file(file!()),
            line!(),
            $crate::debug::extract_fn_name(core::any::type_name_of_val(&|| {})),
            core::format_args!($msg),
        );
    }};
    ($($arg:tt)*) => {{
        let _ = $crate::debug::debug_log_write_with_loc(
            $crate::debug::short_file(file!()),
            line!(),
            $crate::debug::extract_fn_name(core::any::type_name_of_val(&|| {})),
            core::format_args!($($arg)*),
        );
    }};
}

#[macro_export]
macro_rules! measure_time {
    ($b:expr) => {{
        let start = std::time::Instant::now();
        let result = $b;
        $crate::debug_log!("elapsed {:?}", start.elapsed());
        result
    }};
}

#[cfg(not(debug_assertions))]
#[macro_export]
macro_rules! debug_log {
    () => {{}};
    ($msg:expr) => {{}};
    ($($arg:tt)*) => {{}};
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::any;

    #[test]
    fn short_file_last_two_components() {
        assert_eq!(short_file("a/b/c/d.rs"), "c/d.rs");
        assert_eq!(short_file("foo/bar.rs"), "foo/bar.rs");
        assert_eq!(short_file("mod.rs"), "mod.rs");
    }

    #[test]
    fn extract_fn_name_from_closure_typename() {
        assert_eq!(
            extract_fn_name(any::type_name_of_val(&|| {})),
            "extract_fn_name_from_closure_typename"
        );
    }

    #[test]
    fn extract_fn_generic() {
        assert_eq!(
            extract_fn_name("fluentbase_contracts_evm::main_entry<fluentbase_sdk::shared::SharedContextImpl<fluentbase_types::rwasm_context::RwasmContext>>::{{closure}}"),
            "main_entry"
        );
    }
}
