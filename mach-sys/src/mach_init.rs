#[inline]
pub fn mach_task_self() -> mach_port_t {
    unsafe { mach_task_self_ }
}