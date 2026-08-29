use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct SandboxPending<T> {
    evaluation: EvaluationId,
    receiver: mpsc::Receiver<Result<T, SandboxError>>,
}

impl<T> SandboxPending<T> {
    /// Creates a provider result tied to the evaluation allocated by the
    /// Kernel. External providers send exactly one terminal result through the
    /// receiver; dropping the sender is reported as a transport failure.
    pub const fn new(
        evaluation: EvaluationId,
        receiver: mpsc::Receiver<Result<T, SandboxError>>,
    ) -> Self {
        Self {
            evaluation,
            receiver,
        }
    }

    pub const fn evaluation(&self) -> EvaluationId {
        self.evaluation
    }

    pub fn wait(self) -> Result<T, SandboxError> {
        self.receiver.recv().unwrap_or_else(|_| {
            Err(SandboxError::new(
                SandboxErrorCode::TransportFailed,
                "sandbox provider dropped the evaluation result",
            ))
        })
    }
}

/// Provider-side live sandbox. Implementations own backend launch, execution,
/// cancellation, and termination details.
pub trait SandboxInstance {
    fn eval(
        &mut self,
        evaluation: EvaluationId,
        source: String,
    ) -> Result<SandboxPending<String>, SandboxError>;
    fn call(
        &mut self,
        evaluation: EvaluationId,
        callable: String,
        arguments_hta: Vec<u8>,
    ) -> Result<SandboxPending<Vec<u8>>, SandboxError>;
    fn cancel(&mut self, evaluation: EvaluationId) -> Result<bool, SandboxError>;
    fn active_evaluation(&self) -> Option<EvaluationId>;
    fn state(&self) -> SandboxState;
    fn error(&self) -> Option<SandboxError>;
    fn close(&mut self) -> Result<(), SandboxError>;
}

pub trait SandboxProvider {
    fn name(&self) -> &str;
    fn secure(&self) -> bool;
    fn open(&self, spec: &ResolvedSandboxSpec) -> Result<Box<dyn SandboxInstance>, SandboxError>;
}

#[derive(Clone, Debug)]
pub struct ResolvedSandboxBundle {
    pub reference: SandboxBundleReference,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ResolvedSandboxMount {
    pub id: SessionMountId,
    pub kind: String,
    pub key: String,
}

#[derive(Clone, Debug)]
pub struct ResolvedSandboxSpec {
    pub spec: SandboxSpec,
    pub bundles: Vec<ResolvedSandboxBundle>,
    pub mount: Option<ResolvedSandboxMount>,
}

/// Conformance-only provider. Runtime separation is logical and is not a
/// security boundary.
#[derive(Default)]
pub struct InProcessSandboxProvider;

impl SandboxProvider for InProcessSandboxProvider {
    fn name(&self) -> &str {
        "in-process"
    }

    fn secure(&self) -> bool {
        false
    }

    fn open(
        &self,
        resolved: &ResolvedSandboxSpec,
    ) -> Result<Box<dyn SandboxInstance>, SandboxError> {
        resolved.spec.validate()?;
        InProcessSandbox::open(resolved.clone()).map(|instance| Box::new(instance) as _)
    }
}

#[derive(Clone)]
struct ActiveEvaluation {
    id: EvaluationId,
    cancelled: Arc<AtomicBool>,
}

struct WorkerState {
    state: SandboxState,
    active: Option<ActiveEvaluation>,
    error: Option<SandboxError>,
}

enum SandboxCommand {
    Eval {
        evaluation: EvaluationId,
        source: String,
        cancelled: Arc<AtomicBool>,
        reply: mpsc::Sender<Result<String, SandboxError>>,
    },
    Call {
        evaluation: EvaluationId,
        callable: String,
        arguments_hta: Vec<u8>,
        cancelled: Arc<AtomicBool>,
        reply: mpsc::Sender<Result<Vec<u8>, SandboxError>>,
    },
    Close,
}

struct InProcessSandbox {
    commands: mpsc::Sender<SandboxCommand>,
    worker: Option<JoinHandle<()>>,
    shared: Arc<Mutex<WorkerState>>,
    limits: SandboxLimits,
}

impl InProcessSandbox {
    fn open(resolved: ResolvedSandboxSpec) -> Result<Self, SandboxError> {
        let spec = resolved.spec;
        let limits = spec.limits.clone();
        let (commands, receiver) = mpsc::channel();
        let shared = Arc::new(Mutex::new(WorkerState {
            state: SandboxState::Open,
            active: None,
            error: None,
        }));
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("hara-in-process-sandbox".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(move || sandbox_worker(spec, receiver, worker_shared))
            .map_err(|error| {
                SandboxError::new(SandboxErrorCode::ProviderFailed, error.to_string())
            })?;
        Ok(Self {
            commands,
            worker: Some(worker),
            shared,
            limits,
        })
    }

    fn begin(&self, evaluation: EvaluationId) -> Result<Arc<AtomicBool>, SandboxError> {
        let mut shared = self.shared.lock().expect("sandbox state poisoned");
        if shared.state != SandboxState::Open || shared.active.is_some() {
            return Err(if shared.state == SandboxState::Running {
                SandboxError::new(SandboxErrorCode::Busy, "sandbox is busy")
            } else {
                SandboxError::new(
                    SandboxErrorCode::Closed,
                    "sandbox is terminal and cannot be reused",
                )
            });
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        shared.state = SandboxState::Running;
        shared.error = None;
        shared.active = Some(ActiveEvaluation {
            id: evaluation,
            cancelled: Arc::clone(&cancelled),
        });
        Ok(cancelled)
    }

    fn send(&self, command: SandboxCommand) -> Result<(), SandboxError> {
        self.commands.send(command).map_err(|_| {
            SandboxError::new(
                SandboxErrorCode::TransportFailed,
                "sandbox provider command channel is closed",
            )
        })
    }
}

fn sandbox_worker(
    spec: SandboxSpec,
    commands: mpsc::Receiver<SandboxCommand>,
    shared: Arc<Mutex<WorkerState>>,
) {
    let session_spec = match SessionId::parse("SANDBOX") {
        Ok(id) => SessionSpec::new(id, SessionAuthorityPolicy::ZERO),
        Err(error) => {
            finish_provider_failure(&shared, error);
            return;
        }
    };
    let mut runtime = Runtime::sandbox();
    runtime.use_namespace(&spec.entry_namespace);
    let mut session = Session::open(session_spec, runtime);
    while let Ok(command) = commands.recv() {
        match command {
            SandboxCommand::Eval {
                evaluation,
                source,
                cancelled,
                reply,
            } => {
                let result = run_controlled(evaluation, &spec.limits, &cancelled, || {
                    session.eval(&source)
                })
                .and_then(|result| {
                    if result.len() > spec.limits.result_bytes {
                        Err(SandboxError::new(
                            SandboxErrorCode::LimitExceeded,
                            "sandbox result limit exceeded",
                        ))
                    } else {
                        Ok(result)
                    }
                });
                finish_evaluation(&shared, evaluation, &result);
                let _ = reply.send(result);
            }
            SandboxCommand::Call {
                evaluation,
                callable,
                arguments_hta,
                cancelled,
                reply,
            } => {
                let result = run_controlled(evaluation, &spec.limits, &cancelled, || {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        session
                            .runtime_mut()?
                            .invoke_hta(&callable, &arguments_hta)
                            .map_err(|error| error.to_string())
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let _ = (&callable, &arguments_hta);
                        Err::<Vec<u8>, String>(
                            "sandbox HTA calls are unavailable in browser WASM".into(),
                        )
                    }
                })
                .and_then(|result| {
                    if result.len() > spec.limits.result_bytes {
                        Err(SandboxError::new(
                            SandboxErrorCode::LimitExceeded,
                            "sandbox result limit exceeded",
                        ))
                    } else {
                        Ok(result)
                    }
                });
                finish_evaluation(&shared, evaluation, &result);
                let _ = reply.send(result);
            }
            SandboxCommand::Close => break,
        }
    }
    session.release();
}

fn run_controlled<T>(
    _evaluation: EvaluationId,
    limits: &SandboxLimits,
    cancelled: &Arc<AtomicBool>,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, SandboxError> {
    let started = Instant::now();
    let deadline = Duration::from_millis(limits.evaluation_ms);
    let cancellation = Arc::clone(cancelled);
    let result = core::with_evaluation_interrupt(
        Rc::new(move || {
            if cancellation.load(Ordering::Acquire) {
                Some("SANDBOX_CANCELLED".into())
            } else if started.elapsed() >= deadline {
                Some("SANDBOX_TIMEOUT".into())
            } else {
                None
            }
        }),
        operation,
    );
    result.map_err(|error| {
        if error.contains("SANDBOX_CANCELLED") {
            SandboxError::new(SandboxErrorCode::Cancelled, "sandbox evaluation cancelled")
        } else if error.contains("SANDBOX_TIMEOUT") {
            SandboxError::new(SandboxErrorCode::Timeout, "sandbox evaluation timed out")
        } else if error.contains("SESSION_TRANSFER_REJECTED")
            || error.contains("invoke-hta/result-unsupported")
        {
            SandboxError::new(
                SandboxErrorCode::ResultNotTransferable,
                "sandbox result is not transferable",
            )
        } else {
            SandboxError::new(SandboxErrorCode::EvaluationFailed, error)
        }
    })
}

fn finish_evaluation<T>(
    shared: &Arc<Mutex<WorkerState>>,
    evaluation: EvaluationId,
    result: &Result<T, SandboxError>,
) {
    let mut shared = shared.lock().expect("sandbox state poisoned");
    if !shared
        .active
        .as_ref()
        .is_some_and(|active| active.id == evaluation)
    {
        return;
    }
    shared.active = None;
    match result {
        Ok(_) => shared.state = SandboxState::Open,
        Err(error) => {
            shared.state = match error.code {
                SandboxErrorCode::Cancelled => SandboxState::Cancelled,
                _ => SandboxState::Failed,
            };
            shared.error = Some(error.clone());
        }
    }
}

fn finish_provider_failure(shared: &Arc<Mutex<WorkerState>>, message: String) {
    let error = SandboxError::new(SandboxErrorCode::ProviderFailed, message);
    let mut shared = shared.lock().expect("sandbox state poisoned");
    shared.state = SandboxState::Failed;
    shared.error = Some(error);
    shared.active = None;
}

impl SandboxInstance for InProcessSandbox {
    fn eval(
        &mut self,
        evaluation: EvaluationId,
        source: String,
    ) -> Result<SandboxPending<String>, SandboxError> {
        if source.len() > self.limits.source_bytes {
            return Err(SandboxError::new(
                SandboxErrorCode::LimitExceeded,
                "sandbox source limit exceeded",
            ));
        }
        let cancelled = self.begin(evaluation)?;
        let (reply, receiver) = mpsc::channel();
        self.send(SandboxCommand::Eval {
            evaluation,
            source,
            cancelled,
            reply,
        })?;
        Ok(SandboxPending {
            evaluation,
            receiver,
        })
    }

    fn call(
        &mut self,
        evaluation: EvaluationId,
        callable: String,
        arguments_hta: Vec<u8>,
    ) -> Result<SandboxPending<Vec<u8>>, SandboxError> {
        let cancelled = self.begin(evaluation)?;
        let (reply, receiver) = mpsc::channel();
        self.send(SandboxCommand::Call {
            evaluation,
            callable,
            arguments_hta,
            cancelled,
            reply,
        })?;
        Ok(SandboxPending {
            evaluation,
            receiver,
        })
    }

    fn cancel(&mut self, evaluation: EvaluationId) -> Result<bool, SandboxError> {
        let mut shared = self.shared.lock().expect("sandbox state poisoned");
        let Some(active) = shared.active.as_ref() else {
            return Ok(false);
        };
        if active.id != evaluation {
            return Ok(false);
        }
        active.cancelled.store(true, Ordering::Release);
        shared.state = SandboxState::Cancelling;
        Ok(true)
    }

    fn active_evaluation(&self) -> Option<EvaluationId> {
        self.shared
            .lock()
            .expect("sandbox state poisoned")
            .active
            .as_ref()
            .map(|active| active.id)
    }

    fn state(&self) -> SandboxState {
        self.shared.lock().expect("sandbox state poisoned").state
    }

    fn error(&self) -> Option<SandboxError> {
        self.shared
            .lock()
            .expect("sandbox state poisoned")
            .error
            .clone()
    }

    fn close(&mut self) -> Result<(), SandboxError> {
        if let Some(active) = self.active_evaluation() {
            let _ = self.cancel(active)?;
        }
        let _ = self.commands.send(SandboxCommand::Close);
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| {
                SandboxError::new(
                    SandboxErrorCode::ProviderFailed,
                    "sandbox provider worker panicked",
                )
            })?;
        }
        Ok(())
    }
}

struct Sandbox {
    id: SandboxId,
    provider: String,
    secure: bool,
    mount: Option<SessionMountId>,
    next_evaluation_id: u64,
    instance: Box<dyn SandboxInstance>,
}

impl Sandbox {
    fn allocate_evaluation(&mut self) -> EvaluationId {
        let id = EvaluationId::new(self.next_evaluation_id);
        self.next_evaluation_id = self
            .next_evaluation_id
            .checked_add(1)
            .expect("sandbox evaluation identifiers exhausted");
        id
    }
}

impl SessionKernel {
    pub fn register_sandbox_provider(&mut self, provider: Rc<dyn SandboxProvider>) {
        self.sandbox_provider_registry
            .entries
            .insert(provider.name().into(), provider);
    }

    pub fn open_sandbox(&mut self, spec: SandboxSpec) -> Result<SandboxId, SandboxError> {
        spec.validate()?;
        let provider = self
            .sandbox_provider_registry
            .entries
            .get(&spec.provider)
            .ok_or_else(|| {
                SandboxError::new(SandboxErrorCode::ProviderNotFound, spec.provider.clone())
            })?
            .clone();
        let bundles = spec
            .bundles()
            .iter()
            .map(|reference| {
                self.bundle_catalog
                    .entries
                    .get(&reference.digest)
                    .cloned()
                    .map(|bytes| (reference, bytes))
                    .ok_or_else(|| {
                        SandboxError::new(
                            SandboxErrorCode::BundleNotFound,
                            reference.digest.clone(),
                        )
                    })
                    .and_then(|(reference, bytes)| {
                        let actual = format!("sha256:{:x}", Sha256::digest(&bytes));
                        if actual == reference.digest {
                            Ok(ResolvedSandboxBundle {
                                reference: reference.clone(),
                                bytes,
                            })
                        } else {
                            Err(SandboxError::new(
                                SandboxErrorCode::BundleDigestMismatch,
                                reference.digest.clone(),
                            ))
                        }
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mount = spec
            .mount()
            .map(|id| {
                self.mount_registry
                    .entries
                    .get(&id.get())
                    .map(|mount| ResolvedSandboxMount {
                        id,
                        kind: mount.kind.into(),
                        key: mount.key.clone(),
                    })
                    .ok_or_else(|| {
                        SandboxError::new(SandboxErrorCode::MountNotFound, id.to_string())
                    })
            })
            .transpose()?;
        let secure = provider.secure();
        let id = SandboxId(self.sandbox_registry.next_id);
        self.sandbox_registry.next_id = self
            .sandbox_registry
            .next_id
            .checked_add(1)
            .expect("sandbox identifiers exhausted");
        if let Some(mount) = &mount {
            self.mount_registry
                .entries
                .get_mut(&mount.id.get())
                .expect("resolved mount remains registered")
                .attachments += 1;
            self.mount_registry
                .sandbox_attachments
                .insert(id.get(), mount.id.get());
        }
        let resolved = ResolvedSandboxSpec {
            spec: spec.clone(),
            bundles,
            mount,
        };
        let instance = match provider.open(&resolved) {
            Ok(instance) => instance,
            Err(error) => {
                self.release_sandbox_mount(id, spec.mount());
                return Err(error);
            }
        };
        self.sandbox_registry.entries.insert(
            id.get(),
            Sandbox {
                id,
                provider: spec.provider.clone(),
                secure,
                mount: spec.mount(),
                next_evaluation_id: 1,
                instance,
            },
        );
        Ok(id)
    }

    fn release_sandbox_mount(&mut self, id: SandboxId, mount: Option<SessionMountId>) {
        let Some(mount) = mount else {
            return;
        };
        if self.mount_registry.sandbox_attachments.remove(&id.get()) == Some(mount.get()) {
            if let Some(entry) = self.mount_registry.entries.get_mut(&mount.get()) {
                entry.attachments = entry.attachments.saturating_sub(1);
            }
        }
    }

    fn sandbox_mut(&mut self, id: SandboxId) -> Result<&mut Sandbox, SandboxError> {
        self.sandbox_registry
            .entries
            .get_mut(&id.get())
            .ok_or_else(|| SandboxError::new(SandboxErrorCode::NotFound, id.to_string()))
    }

    pub fn sandbox_eval(
        &mut self,
        id: SandboxId,
        source: &str,
    ) -> Result<SandboxPending<String>, SandboxError> {
        let sandbox = self.sandbox_mut(id)?;
        let evaluation = sandbox.allocate_evaluation();
        sandbox.instance.eval(evaluation, source.to_owned())
    }

    pub fn sandbox_call(
        &mut self,
        id: SandboxId,
        callable: &str,
        arguments_hta: &[u8],
    ) -> Result<SandboxPending<Vec<u8>>, SandboxError> {
        let sandbox = self.sandbox_mut(id)?;
        let evaluation = sandbox.allocate_evaluation();
        sandbox
            .instance
            .call(evaluation, callable.to_owned(), arguments_hta.to_vec())
    }

    pub fn cancel_sandbox(&mut self, id: SandboxId) -> Result<bool, SandboxError> {
        let sandbox = self.sandbox_mut(id)?;
        let Some(evaluation) = sandbox.instance.active_evaluation() else {
            return Ok(false);
        };
        sandbox.instance.cancel(evaluation)
    }

    pub fn cancel_sandbox_evaluation(
        &mut self,
        id: SandboxId,
        evaluation: EvaluationId,
    ) -> Result<bool, SandboxError> {
        self.sandbox_mut(id)?.instance.cancel(evaluation)
    }

    pub fn sandbox_status(&self, id: SandboxId) -> Result<SandboxStatus, SandboxError> {
        let sandbox = self
            .sandbox_registry
            .entries
            .get(&id.get())
            .ok_or_else(|| SandboxError::new(SandboxErrorCode::NotFound, id.to_string()))?;
        Ok(SandboxStatus {
            id: sandbox.id,
            provider: sandbox.provider.clone(),
            state: sandbox.instance.state(),
            secure: sandbox.secure,
            evaluation_active: sandbox.instance.active_evaluation().is_some(),
            error: sandbox.instance.error(),
        })
    }

    pub fn close_sandbox(&mut self, id: SandboxId) -> Result<(), SandboxError> {
        let mut sandbox = self
            .sandbox_registry
            .entries
            .remove(&id.get())
            .ok_or_else(|| SandboxError::new(SandboxErrorCode::NotFound, id.to_string()))?;
        let result = sandbox.instance.close();
        self.release_sandbox_mount(id, sandbox.mount);
        result
    }
}
