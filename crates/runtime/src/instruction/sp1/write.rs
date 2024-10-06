use rwasm::Caller;
use rwasm::core::Trap;
use crate::RuntimeContext;

pub struct SyscallWrite;

impl SyscallWrite {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        fd: u32,
        write_buf: u32,
        length: u32,
    ) -> Result<(), Trap> {

        // Read nbytes from memory starting at write_buf.
        // let bytes =  caller.read_memory(write_buf, length)?.to_vec();
        // let slice = bytes.as_slice();
        // if fd == 1 {
        //     let s = core::str::from_utf8(slice).unwrap();
        //
        //     let flush_s = update_io_buf(ctx, fd, s);
        //     if !flush_s.is_empty() {
        //         flush_s.into_iter().for_each(|line| println!("stdout: {}", line));
        //     }
        //
        // } else if fd == 2 {
        //     let s = core::str::from_utf8(slice).unwrap();
        //     let flush_s = update_io_buf(ctx, fd, s);
        //     if !flush_s.is_empty() {
        //         flush_s.into_iter().for_each(|line| println!("stderr: {}", line));
        //     }
        // } else if fd == 3 {
        //     rt.state.public_values_stream.extend_from_slice(slice);
        // } else if fd == 4 {
        //     rt.state.input_stream.push(slice.to_vec());
        // } else if let Some(mut hook) = rt.hook_registry.get(fd) {
        //     let res = hook.invoke_hook(rt.hook_env(), slice);
        //     // Add result vectors to the beginning of the stream.
        //     let ptr = rt.state.input_stream_ptr;
        //     rt.state.input_stream.splice(ptr..ptr, res);
        // }
        Ok(())
    }
}


// #[allow(clippy::mut_mut)]
// fn update_io_buf(mut caller: Caller<'_, RuntimeContext>, fd: u32, s: &str) -> Vec<String> {
//     let rt = &mut ctx.rt;
//     let entry = rt.io_buf.entry(fd).or_default();
//     entry.push_str(s);
//     if entry.contains('\n') {
//         // Return lines except for the last from buf.
//         let prev_buf = std::mem::take(entry);
//         let mut lines = prev_buf.split('\n').collect::<Vec<&str>>();
//         let last = lines.pop().unwrap_or("");
//         *entry = last.to_string();
//         lines.into_iter().map(std::string::ToString::to_string).collect::<Vec<String>>()
//     } else {
//         vec![]
//     }
// }
