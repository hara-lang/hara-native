use hara_wasm::core::Value;
use hara_wasm::{
    restricted_sandbox_runtime, restricted_sandbox_runtime_with_host, EvaluationId,
    ResolvedSandboxSpec, SandboxError, SandboxErrorCode, SandboxInstance, SandboxPending,
    SandboxProvider, SandboxSpec, SandboxState, SessionKernel,
};
use std::rc::Rc;
use std::sync::mpsc;

struct ExternalProvider;

impl SandboxProvider for ExternalProvider {
    fn name(&self) -> &str {
        "external-test"
    }

    fn secure(&self) -> bool {
        true
    }

    fn open(
        &self,
        resolved: &ResolvedSandboxSpec,
    ) -> Result<Box<dyn SandboxInstance>, SandboxError> {
        resolved.spec.validate()?;
        assert_eq!(resolved.spec.runtime(), "hara.standard/0-alpha");
        Ok(Box::new(ExternalInstance {
            state: SandboxState::Open,
            active: None,
        }))
    }
}

struct ExternalInstance {
    state: SandboxState,
    active: Option<EvaluationId>,
}

impl ExternalInstance {
    fn completed<T>(
        &mut self,
        evaluation: EvaluationId,
        result: Result<T, SandboxError>,
    ) -> SandboxPending<T> {
        assert!(evaluation.get() > 0);
        let (sender, receiver) = mpsc::channel();
        sender.send(result).unwrap();
        SandboxPending::new(evaluation, receiver)
    }
}

impl SandboxInstance for ExternalInstance {
    fn eval(
        &mut self,
        evaluation: EvaluationId,
        source: String,
    ) -> Result<SandboxPending<String>, SandboxError> {
        if source == "fail" {
            return Err(SandboxError::new(
                SandboxErrorCode::EvaluationFailed,
                "external evaluator rejected source",
            ));
        }
        Ok(self.completed(evaluation, Ok(source)))
    }

    fn call(
        &mut self,
        evaluation: EvaluationId,
        _callable: String,
        arguments_hta: Vec<u8>,
    ) -> Result<SandboxPending<Vec<u8>>, SandboxError> {
        Ok(self.completed(evaluation, Ok(arguments_hta)))
    }

    fn cancel(&mut self, evaluation: EvaluationId) -> Result<bool, SandboxError> {
        Ok(self.active == Some(evaluation))
    }

    fn active_evaluation(&self) -> Option<EvaluationId> {
        self.active
    }

    fn state(&self) -> SandboxState {
        self.state
    }

    fn error(&self) -> Option<SandboxError> {
        None
    }

    fn close(&mut self) -> Result<(), SandboxError> {
        self.state = SandboxState::Closed;
        Ok(())
    }
}

#[test]
fn external_crate_can_implement_the_complete_provider_contract() {
    let mut kernel = SessionKernel::new();
    kernel.register_sandbox_provider(Rc::new(ExternalProvider));
    let spec = SandboxSpec::new(
        "hara.sandbox/0-alpha",
        "external-test",
        "hara.standard/0-alpha",
        "user",
        Default::default(),
    )
    .unwrap();
    let sandbox = kernel.open_sandbox(spec).unwrap();

    let evaluation = kernel.sandbox_eval(sandbox, "(+ 1 2)").unwrap();
    assert!(evaluation.evaluation().get() > 0);
    assert_eq!(evaluation.wait().unwrap(), "(+ 1 2)");

    let failure = kernel.sandbox_eval(sandbox, "fail").unwrap_err();
    assert_eq!(failure.code, SandboxErrorCode::EvaluationFailed);
    kernel.close_sandbox(sandbox).unwrap();
}

#[test]
fn external_crate_can_construct_only_the_restricted_runtime_profile() {
    let mut runtime = restricted_sandbox_runtime();
    assert_eq!(
        runtime.eval_native_value("(+ 1 2)").unwrap(),
        Value::Number(3)
    );

    for forbidden in [
        "Runtime/current",
        "Kernel/current",
        "Sandbox/open",
        "Package/install",
        "File/exists?",
        "Socket/connect",
        "Process/exec",
        "Host/call",
        "std.native.Host/call",
    ] {
        let error = runtime.eval_native_value(forbidden).unwrap_err();
        assert!(
            error.contains("unbound symbol"),
            "restricted Runtime unexpectedly resolved {forbidden}: {error}"
        );
    }
}

#[test]
fn external_crate_can_inject_only_one_fully_qualified_host_call() {
    let mut runtime =
        restricted_sandbox_runtime_with_host(Rc::new(|service, method, arguments| {
            if service == "hoplite.console" && method == "status" && arguments.is_empty() {
                Ok(Value::Number(42))
            } else {
                Err("sandbox host service denied".into())
            }
        }));
    assert_eq!(
        runtime
            .eval_native_value("(deref (std.native.Host/call \"hoplite.console\" \"status\" []))",)
            .unwrap(),
        Value::Number(42)
    );

    for forbidden in [
        "Host/call",
        "std.native.Host/describe",
        "std.native.Host/capabilities",
        "std.native.Host/capability?",
        "Runtime/current",
        "Kernel/current",
        "Sandbox/open",
    ] {
        let error = runtime.eval_native_value(forbidden).unwrap_err();
        assert!(
            error.contains("unbound symbol"),
            "narrow sandbox host unexpectedly resolved {forbidden}: {error}"
        );
    }
}
