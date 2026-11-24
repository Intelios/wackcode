//! One worker thread per app being driven. Every Accessibility call for an app runs on its
//! thread.
//!
//! Invariants:
//! - AX handles never leave their app's thread (they aren't `Send`); jobs carry plain data in and
//!   out.
//! - A hung app stalls only its own thread: every AX message has a one-second timeout, and a caller
//!   gives up waiting after [`JOB_LIMIT`] while the thread finishes on its own.
//! - An app's ref numbers come from a counter kept outside its thread, so numbers never repeat
//!   when a thread ends and a new one starts (refs from before then simply resolve to nothing).
//! - A thread ends when its app quits, found when it next waits for work.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::AtomicU32,
        mpsc::{self, RecvTimeoutError},
    },
    time::Duration,
};

use crate::native::{
    apps,
    ax::{self, Element, Registry},
};

/// How long a caller waits for one job.
pub const JOB_LIMIT: Duration = Duration::from_secs(20);
/// How often an idle thread checks that its app still runs.
const IDLE_CHECK: Duration = Duration::from_secs(30);

/// What a job gets: the app's AX element and its refs.
pub struct AppThread {
    pub pid: i32,
    pub app: Element,
    pub registry: Registry,
    /// Whether `AXManualAccessibility` has been asked of this app (Electron and Chromium only).
    pub manual: bool,
}

type Job = Box<dyn FnOnce(&mut AppThread) + Send>;

#[derive(Default)]
pub struct Engine {
    lanes: Mutex<HashMap<i32, mpsc::Sender<Job>>>,
    counters: Mutex<HashMap<i32, Arc<AtomicU32>>>,
}

impl Engine {
    fn spawn(&self, pid: i32) -> mpsc::Sender<Job> {
        let (sender, receiver) = mpsc::channel::<Job>();
        let next = self.counters.lock().unwrap_or_else(|e| e.into_inner()).entry(pid).or_default().clone();
        // If the thread can't start, the job is dropped with its channel and the caller says so.
        let _ = std::thread::Builder::new().name(format!("wackcode-computer-{pid}")).spawn(move || {
            let mut thread = AppThread { pid, app: ax::application(pid), registry: Registry::new(next), manual: false };
            loop {
                match receiver.recv_timeout(IDLE_CHECK) {
                    Ok(job) => job(&mut thread),
                    Err(RecvTimeoutError::Timeout) if apps::is_running(pid) => {}
                    Err(_) => break,
                }
            }
        });
        sender
    }

    /// Runs `job` on `pid`'s thread and waits for it, at most [`JOB_LIMIT`].
    pub async fn run<T: Send + 'static>(&self, pid: i32, job: impl FnOnce(&mut AppThread) -> T + Send + 'static) -> Result<T, String> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move |thread| {
            let _ = reply.send(job(thread));
        });
        {
            let mut lanes = self.lanes.lock().unwrap_or_else(|e| e.into_inner());
            let sender = lanes.entry(pid).or_insert_with(|| self.spawn(pid));
            if let Err(mpsc::SendError(job)) = sender.send(job) {
                // The thread ended (its app quit, or it was idle when the app restarted).
                let fresh = self.spawn(pid);
                let _ = fresh.send(job);
                lanes.insert(pid, fresh);
            }
        }
        match tokio::time::timeout(JOB_LIMIT, answer).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err("That app's Accessibility thread stopped; the app may have quit.".into()),
            Err(_) => Err(format!("The app didn't answer Accessibility within {} seconds: it may be busy or hung.", JOB_LIMIT.as_secs())),
        }
    }

    /// Forgets an app's thread (it ends once idle).
    pub fn forget(&self, pid: i32) {
        self.lanes.lock().unwrap_or_else(|e| e.into_inner()).remove(&pid);
    }
}
