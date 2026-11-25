//! Running work on the main thread, from anywhere: AppKit windows (the agent's cursor) live
//! there. Posts closures to the main dispatch queue, which AppKit's run loop drains, so it
//! needs nothing from GPUI. (`wack-browser` has its own copy; the crates don't share code.)

use std::ffi::c_void;

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
}

type Job = Box<dyn FnOnce() + Send + 'static>;

extern "C" fn trampoline(context: *mut c_void) {
    // SAFETY: `context` is the `Box<Job>` leaked by `post`, run exactly once.
    let job = unsafe { Box::from_raw(context.cast::<Job>()) };
    job();
}

pub fn is_main() -> bool {
    // SAFETY: a plain query of the calling thread.
    unsafe { libc::pthread_main_np() == 1 }
}

/// Runs `job` on the main thread, later.
pub fn post(job: impl FnOnce() + Send + 'static) {
    let job: Box<Job> = Box::new(Box::new(job));
    // SAFETY: the main queue lives for the whole process; the trampoline frees the context.
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, Box::into_raw(job).cast(), trampoline) }
}
