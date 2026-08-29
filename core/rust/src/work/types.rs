use super::*;

/// Validated portable identifier for one native work run.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkId(String);

impl WorkId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err("work run ID cannot be blank".into());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Monotonic state of a live native work run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkRunState {
    Queued,
    Running,
    Waiting,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

impl WorkRunState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Non-blocking status snapshot for a live work run.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkRunStatus {
    pub id: WorkId,
    pub state: WorkRunState,
    pub started_at_millis: u64,
    pub finished_at_millis: Option<u64>,
    pub error: Option<PromiseRejection>,
    pub cancel_reason: Option<Value>,
    pub parent_id: Option<WorkId>,
    pub child_count: usize,
    pub deadline_remaining_millis: Option<u64>,
    pub detached: bool,
}

/// Process-host lifecycle metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkHostStatus {
    pub state: &'static str,
    pub run_count: usize,
    pub queued_count: usize,
}

/// Submission-time scope and deadline options.
#[derive(Clone, Debug, Default)]
pub struct WorkOptions {
    pub id: Option<WorkId>,
    pub timeout: Option<Duration>,
    pub deadline: Option<Instant>,
    pub detached: bool,
}

impl WorkOptions {
    pub fn with_id(id: impl Into<String>) -> Result<Self, String> {
        Ok(Self {
            id: Some(WorkId::new(id)?),
            ..Self::default()
        })
    }
}
