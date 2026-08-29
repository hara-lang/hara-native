use crate::work::{process_work_host, WorkHost};

impl crate::SessionKernel {
    /// Return the process/evaluator-thread work host shared by every session.
    pub fn work_host(&self) -> WorkHost {
        process_work_host()
    }
}
