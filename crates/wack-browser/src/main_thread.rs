//! Running work on the main thread, from anywhere.
//!
//! CEF runs with an external message pump, so its UI thread *is* the app's main thread (the one
//! GPUI runs AppKit on). Every CEF call that touches a browser must happen there; the agent's
//! tools run on Tokio. This posts closures to the main dispatch queue, which AppKit's run loop
//! drains, so it works without knowing anything about GPUI.

use std::ffi::c_void;

use tokio::sync::oneshot;

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn dispatch_after_f(
        when: u64,
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn dispatch_time(when: u64, delta: i64) -> u64;
}

type Job = Box<dyn FnOnce() + Send + 'static>;

extern "C" fn trampoline(context: *mut c_void) {
    // SAFETY: `context` is the `Box<Job>` leaked by `post`/`post_after`, run exactly once.
    let job = unsafe { Box::from_raw(context.cast::<Job>()) };
    job();
}

fn leak(job: impl FnOnce() + Send + 'static) -> *mut c_void {
    let job: Box<Job> = Box::new(Box::new(job));
    Box::into_raw(job).cast()
}

pub fn is_main() -> bool {
    // SAFETY: a plain query of the calling thread.
    unsafe { libc::pthread_main_np() == 1 }
}

/// Runs `job` on the main thread, later.
pub fn post(job: impl FnOnce() + Send + 'static) {
    // SAFETY: the main queue lives for the whole process; the trampoline frees the context.
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, leak(job), trampoline) }
}

/// Runs `job` on the main thread after `delay_ms`.
pub fn post_after(delay_ms: i64, job: impl FnOnce() + Send + 'static) {
    // SAFETY: as in `post`.
    unsafe {
        dispatch_after_f(
            dispatch_time(0, delay_ms.max(0) * 1_000_000),
            &raw const _dispatch_main_q,
            leak(job),
            trampoline,
        )
    }
}

/// Runs `job` on the main thread and waits for its result without blocking a thread.
pub async fn call<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (send, receive) = oneshot::channel();
    post(move || {
        let _ = send.send(job());
    });
    receive
        .await
        .map_err(|_| "The browser stopped before it could answer.".to_string())
}
