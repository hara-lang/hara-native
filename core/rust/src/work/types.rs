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

/// A portable absolute deadline on the runtime's monotonic nanosecond clock.
///
/// Unlike `std::time::Instant`, this remains available in browser Wasm where
/// the native standard-library clock is intentionally absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkDeadline(u64);

impl WorkDeadline {
    pub fn at_monotonic_nanos(value: u64) -> Self {
        Self(value)
    }

    pub fn after(timeout: Duration) -> Self {
        let timeout = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
        Self(monotonic_nanos().saturating_add(timeout))
    }

    pub fn monotonic_nanos(self) -> u64 {
        self.0
    }

    pub fn expired(self) -> bool {
        monotonic_nanos() >= self.0
    }

    pub fn remaining_millis(self) -> u64 {
        self.0
            .saturating_sub(monotonic_nanos())
            .saturating_div(1_000_000)
    }
}

/// Submission-time scope and deadline options.
#[derive(Clone, Debug, Default)]
pub struct WorkOptions {
    pub id: Option<WorkId>,
    pub timeout: Option<Duration>,
    pub deadline: Option<WorkDeadline>,
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
