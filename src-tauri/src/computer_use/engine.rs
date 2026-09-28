//! The engine thread: one dedicated thread makes every Accessibility call, posts every input
//! event and owns every element handle (`AXUIElement` is not `Send`). A single thread also
//! makes pointer actions globally exclusive, so two chats can never fight over the cursor.
//!
//! The async side sends it closures and awaits their results. It never blocks the main thread
//! on this one; this one may wait on the main thread.

use super::ax::Element;
use super::geometry::Rect;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use tokio::sync::oneshot;

type Job = Box<dyn FnOnce(&mut EngineState) + Send + 'static>;

/// What a screenshot recorded, for mapping its pixels back to the screen.
#[derive(Debug, Clone, Copy)]
pub struct CaptureGeometry {
    pub frame: Rect,
    pub output: (u32, u32),
}

/// The latest observation of one window: its refs (from the last snapshot) and capture
/// geometry (from the last screenshot). A new observation of the same window replaces it.
pub struct Observation {
    pub id: String,
    pub pid: i32,
    pub window: Element,
    /// The snapshot the refs came from (`e{n}`), when there has been one.
    pub ref_prefix: Option<String>,
    pub refs: Vec<Element>,
    pub capture: Option<CaptureGeometry>,
}

#[derive(Default)]
pub struct EngineState {
    /// Per chat, the latest observation of each (pid, window number).
    pub chats: HashMap<String, HashMap<(i32, u32), Observation>>,
    counter: u64,
}

impl EngineState {
    pub fn next_id(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    pub fn observation(&self, task_id: &str, state_id: &str) -> Option<&Observation> {
        self.chats.get(task_id)?.values().find(|observation| observation.id == state_id)
    }
}

/// Set when the user stops computer use (Stop, the hotkey, the menu bar) or the request is
/// cancelled; long jobs check it between steps.
#[derive(Clone, Default)]
pub struct StopToken(Arc<AtomicBool>);

impl StopToken {
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct Engine {
    sender: Mutex<mpsc::Sender<Job>>,
}

impl Engine {
    pub fn start() -> Self {
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("wackcode-computer-use".into())
            .spawn(move || {
                let mut state = EngineState::default();
                while let Ok(job) = receiver.recv() {
                    // A panicking job drops its reply (the caller reports it); the thread lives on.
                    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| job(&mut state)));
                }
            })
            .expect("the computer-use thread could not start");
        Self { sender: Mutex::new(sender) }
    }

    /// Queues a job without waiting for it (cleanup).
    pub fn submit(&self, job: impl FnOnce(&mut EngineState) + Send + 'static) {
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(Box::new(job));
        }
    }

    pub async fn run<T: Send + 'static>(&self, job: impl FnOnce(&mut EngineState) -> T + Send + 'static) -> Result<T, String> {
        let (reply, receiver) = oneshot::channel();
        self.sender
            .lock()
            .map_err(|_| "Computer use is unavailable.".to_string())?
            .send(Box::new(move |state| {
                let _ = reply.send(job(state));
            }))
            .map_err(|_| "Computer use is unavailable.".to_string())?;
        receiver.await.map_err(|_| "Computer use failed unexpectedly.".to_string())
    }
}
