//! The ledger's view of the loop: one report per model invocation, handed to whoever is
//! listening. The writer itself lives in the engine (`wack_core::usage`); the loop only says
//! what happened — which purpose the call served, how long it took, how it ended, and what
//! counts the provider reported.
//!
//! Invariants:
//! - One report per model invocation. Retries inside one request (a rate limit, a dropped
//!   connection) are one invocation, as version 2 counted them.
//! - `duration_ms` covers the whole invocation, first attempt to last token.

use std::{sync::Arc, time::Instant};

use wack_llm::Usage;

/// Why a model was called. A sub-agent's calls carry its id as well ([`ModelCall::subagent_id`]),
/// whatever their purpose: its compactions stay `Compaction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// The conversation's own requests.
    Chat,
    /// A sub-agent's own requests: the `Chat` of a child.
    Subagent,
    /// A compaction's summary request.
    Compaction,
    /// An automatic chat title.
    Title,
    /// A settled group of steps' one-line label, written by its own small model.
    StepSummary,
}

impl Purpose {
    /// Every purpose, for checking each against the ledger's contract.
    pub const ALL: [Purpose; 5] = [Purpose::Chat, Purpose::Subagent, Purpose::Compaction, Purpose::Title, Purpose::StepSummary];

    /// The ledger's spelling, part of its published contract.
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Chat => "chat",
            Purpose::Subagent => "subagent",
            Purpose::Compaction => "compaction",
            Purpose::Title => "title",
            Purpose::StepSummary => "step_summary",
        }
    }
}

/// How a model invocation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallOutcome {
    /// The provider answered.
    Completed,
    /// The provider refused or failed the request.
    Failed,
    /// The call was abandoned before it finished: the user stopped the run, or a timeout cut it.
    Cancelled,
}

impl CallOutcome {
    /// The ledger's spelling, part of its published contract.
    pub fn as_str(self) -> &'static str {
        match self {
            CallOutcome::Completed => "completed",
            CallOutcome::Failed => "failed",
            CallOutcome::Cancelled => "cancelled",
        }
    }
}

/// One finished model invocation, as the ledger wants it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCall {
    pub purpose: Purpose,
    /// The model id that was sent.
    pub model: String,
    pub duration_ms: u64,
    pub outcome: CallOutcome,
    /// The provider's own counts, or `None` when it exposed none usable. Never an estimate.
    pub usage: Option<Usage>,
    /// The sub-agent that made the call, or `None` for the chat itself.
    pub subagent_id: Option<String>,
}

/// Receives the loop's model calls. The engine's ledger writer implements this.
pub trait Recorder: Send + Sync {
    fn record(&self, call: ModelCall);
}

/// Reports one finished call to the recorder, when there is one.
pub(crate) fn report(
    recorder: &Option<Arc<dyn Recorder>>,
    purpose: Purpose,
    model: &str,
    started: Instant,
    outcome: CallOutcome,
    usage: Option<Usage>,
) {
    if let Some(recorder) = recorder {
        recorder.record(ModelCall {
            purpose,
            model: model.to_string(),
            duration_ms: started.elapsed().as_millis() as u64,
            outcome,
            usage,
            subagent_id: None,
        });
    }
}

/// A child's recorder: the parent's, with every call marked as the child's. Its own requests
/// become [`Purpose::Subagent`]; other purposes (a compaction) keep their own.
pub struct ForSubagent {
    pub inner: Arc<dyn Recorder>,
    pub id: String,
}

impl Recorder for ForSubagent {
    fn record(&self, mut call: ModelCall) {
        if call.purpose == Purpose::Chat {
            call.purpose = Purpose::Subagent;
        }
        call.subagent_id = Some(self.id.clone());
        self.inner.record(call);
    }
}
