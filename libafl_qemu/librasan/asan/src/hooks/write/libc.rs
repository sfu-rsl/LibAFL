use core::ffi::{CStr, c_char, c_long};

use libc::{SYS_write, c_int, c_void};
use log::trace;

use crate::{
    asan_load, asan_panic, asan_swap, asan_sym, size_t, ssize_t,
    symbols::{AtomicGuestAddr, Function, FunctionPointer},
};

#[derive(Debug)]
struct FunctionSyscall;

impl Function for FunctionSyscall {
    type Func = unsafe extern "C" fn(num: c_long, ...) -> c_long;
    const NAME: &'static CStr = c"syscall";
}

static SYSCALL_ADDR: AtomicGuestAddr = AtomicGuestAddr::new();

/// # Safety
/// See man pages
#[unsafe(export_name = "patch_write")]
pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t {
    unsafe {
        trace!("write - fd: {:#x}, buf: {:p}, count: {:#x}", fd, buf, count);

        if buf.is_null() && count != 0 {
            asan_panic(c"msg is null".as_ptr() as *const c_char);
        }

        asan_load(buf, count);
        let addr = SYSCALL_ADDR
            .get_or_insert_with(|| asan_sym(FunctionSyscall::NAME.as_ptr() as *const c_char));
        let fn_syscall = FunctionSyscall::as_ptr(addr).unwrap();
        asan_swap(false);
        let ret = fn_syscall(SYS_write, fd, buf, count);
        asan_swap(true);
        ret as ssize_t
    }
}
