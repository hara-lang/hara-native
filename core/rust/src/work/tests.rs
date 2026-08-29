use super::*;

#[test]
fn domain_events_accept_only_named_event_types() {
    let host = WorkHost::new();
    let accepted = Rc::new(RefCell::new(Vec::new()));
    let task_accepted = accepted.clone();
    let run = host
        .submit_scoped(
            WorkOptions::with_id("domain-events").unwrap(),
            move |context| {
                task_accepted.borrow_mut().extend([
                    context.emit(Value::Keyword("task/keyword".into()), Value::Number(1)),
                    context.emit(Value::Symbol("task/symbol".into()), Value::Number(2)),
                    context.emit(Value::String("task/string".into()), Value::Number(3)),
                    context.emit(Value::Number(42), Value::Number(4)),
                ]);
                Ok(Value::Keyword("done".into()))
            },
        )
        .unwrap();

    assert_eq!(
        run.work_result().state(),
        PromiseState::Fulfilled(Value::Keyword("done".into()))
    );
    assert_eq!(*accepted.borrow(), vec![true, true, true, false]);
}

#[test]
fn stop_drains_admitted_work_while_kill_cancels_it() {
    let host = WorkHost::new();
    let draining = host
        .submit(Some("stop-drain"), || Ok(Value::Number(7)))
        .unwrap();
    host.stop();
    assert!(host.submit(Some("rejected"), || Ok(Value::Nil)).is_err());
    host.drain();
    assert_eq!(
        draining.work_result().state(),
        PromiseState::Fulfilled(Value::Number(7))
    );

    host.start();
    let cancelled = host
        .submit(Some("kill-cancel"), || Ok(Value::Number(9)))
        .unwrap();
    host.kill();
    assert_eq!(cancelled.work_status().state, WorkRunState::Cancelled);
}
use std::cell::Cell;

#[test]
fn submission_returns_a_queued_handle_before_execution() {
    let host = WorkHost::new();
    let executed = Rc::new(Cell::new(false));
    let task_executed = executed.clone();
    let run = host
        .submit(Some("immediate"), move || {
            task_executed.set(true);
            Ok(Value::Number(7))
        })
        .unwrap();

    assert_eq!(run.work_status().state, WorkRunState::Queued);
    assert!(!executed.get());
    assert_eq!(
        run.work_result().state(),
        PromiseState::Fulfilled(Value::Number(7))
    );
    assert!(executed.get());
    assert_eq!(run.work_status().state, WorkRunState::Completed);
}

#[test]
fn independent_session_kernels_resolve_the_same_live_handle() {
    let first = crate::SessionKernel::new().work_host();
    let second = crate::SessionKernel::new().work_host();
    assert!(first.same_identity(&second));

    let run = first
        .submit(None, || Ok(Value::String("shared".into())))
        .unwrap();
    let resolved = second.resolve_id(&run.work_id()).unwrap();
    assert!(run.same_identity(&resolved));
    assert!(run.work_result().same_identity(&resolved.work_result()));
    assert_eq!(
        resolved.work_result().state(),
        PromiseState::Fulfilled(Value::String("shared".into()))
    );
}

#[test]
fn failure_is_retained_and_rejects_the_cached_result() {
    let host = WorkHost::new();
    let run = host
        .submit(Some("failed"), || Err("executor failed".into()))
        .unwrap();
    let result = run.work_result();
    assert!(result.same_identity(&run.work_result()));
    assert!(matches!(result.state(), PromiseState::Rejected(_)));

    let status = run.work_status();
    assert_eq!(status.state, WorkRunState::Failed);
    let Some(PromiseRejection::Value(Value::Map(fields))) = status.error else {
        panic!("work failure was not retained as a structured value");
    };
    assert_eq!(
        fields.get(&Value::Keyword("code".into())),
        Some(&Value::Keyword("work/failed".into()))
    );
}

#[test]
fn cancelling_queued_work_prevents_its_executor_from_starting() {
    let host = WorkHost::new();
    let executed = Rc::new(Cell::new(false));
    let task_executed = executed.clone();
    let run = host
        .submit(Some("cancelled"), move || {
            task_executed.set(true);
            Ok(Value::Number(1))
        })
        .unwrap();

    assert!(run.cancel(Value::Keyword("test".into())));
    host.drain();
    assert!(!executed.get());
    assert_eq!(run.work_status().state, WorkRunState::Cancelled);
    assert!(matches!(
        run.work_result().state(),
        PromiseState::Rejected(ref error) if error.is_cancelled()
    ));
}

#[test]
fn adopted_promises_settle_status_and_result_once() {
    let host = WorkHost::new();
    let source = Promise::new();
    let task_source = source.clone();
    let run = host
        .submit(Some("adopted"), move || Ok(Value::Promise(task_source)))
        .unwrap();
    let result = run.work_result();

    assert_eq!(result.state(), PromiseState::Pending);
    assert_eq!(run.work_status().state, WorkRunState::Waiting);
    source.resolve(Value::Number(42));
    assert_eq!(run.work_status().state, WorkRunState::Completed);
    assert_eq!(result.state(), PromiseState::Fulfilled(Value::Number(42)));
    assert!(!run.cancel(Value::Keyword("late".into())));
    assert_eq!(result.state(), PromiseState::Fulfilled(Value::Number(42)));
}

#[test]
fn cancellation_cancels_active_promise_and_runs_finalizers_once() {
    let host = WorkHost::new();
    let source = Promise::new();
    let task_source = source.clone();
    let cleanups = Rc::new(Cell::new(0));
    let task_cleanups = cleanups.clone();
    let run = host
        .submit_scoped(
            WorkOptions::with_id("scope-cancel").unwrap(),
            move |context| {
                context.on_close(move |_| {
                    task_cleanups.set(task_cleanups.get() + 1);
                    Ok(())
                });
                Ok(Value::Promise(task_source))
            },
        )
        .unwrap();
    host.run(&run.work_id());
    assert_eq!(run.work_status().state, WorkRunState::Waiting);

    assert!(run.cancel(Value::Keyword("stop".into())));
    assert!(!run.cancel(Value::Keyword("again".into())));
    assert_eq!(cleanups.get(), 1);
    assert_eq!(run.work_status().state, WorkRunState::Cancelled);
    assert!(matches!(source.state(), PromiseState::Rejected(_)));
}

#[test]
fn cancellation_prevents_a_delayed_native_effect_from_running() {
    let host = WorkHost::new();
    let fired = Rc::new(Cell::new(false));
    let task_fired = fired.clone();
    let run = host
        .submit(Some("cancel-delay"), move || {
            let source = Promise::new();
            source.schedule(
                Duration::from_millis(20),
                Rc::new(move || {
                    task_fired.set(true);
                    Ok(Value::Number(42))
                }),
            );
            let adopted = Promise::new();
            adopted.adopt(&source);
            Ok(Value::Promise(adopted))
        })
        .unwrap();
    host.run(&run.work_id());
    assert_eq!(run.work_status().state, WorkRunState::Waiting);

    assert!(run.cancel(Value::Keyword("stop-timer".into())));
    std::thread::sleep(Duration::from_millis(30));

    assert!(!fired.get());
    assert_eq!(run.work_status().state, WorkRunState::Cancelled);
}

#[test]
fn parent_waits_for_attached_child_and_cancellation_flows_downward() {
    let host = WorkHost::new();
    let child_source = Promise::new();
    let task_child_source = child_source.clone();
    let parent = host
        .submit_scoped(WorkOptions::with_id("parent").unwrap(), move |context| {
            context.submit_child(WorkOptions::with_id("child").unwrap(), move |_| {
                Ok(Value::Promise(task_child_source))
            })?;
            Ok(Value::Keyword("parent".into()))
        })
        .unwrap();
    host.run(&parent.work_id());
    host.run_next();
    assert_eq!(parent.work_status().state, WorkRunState::Waiting);
    let child = host.resolve("child").unwrap();
    assert_eq!(child.work_status().parent_id, Some(parent.work_id()));

    assert!(parent.cancel(Value::Keyword("parent-stop".into())));
    assert_eq!(parent.work_status().state, WorkRunState::Cancelled);
    assert_eq!(child.work_status().state, WorkRunState::Cancelled);
    assert!(matches!(child_source.state(), PromiseState::Rejected(_)));
}

#[test]
fn parent_completion_is_released_after_child_completion() {
    let host = WorkHost::new();
    let child_source = Promise::new();
    let task_child_source = child_source.clone();
    let parent = host
        .submit_scoped(
            WorkOptions::with_id("wait-parent").unwrap(),
            move |context| {
                context.submit_child(WorkOptions::with_id("wait-child").unwrap(), move |_| {
                    Ok(Value::Promise(task_child_source))
                })?;
                Ok(Value::Keyword("parent".into()))
            },
        )
        .unwrap();
    host.run(&parent.work_id());
    host.run_next();
    assert_eq!(parent.work_status().state, WorkRunState::Waiting);
    assert_eq!(parent.work_result().state(), PromiseState::Pending);

    child_source.resolve(Value::Keyword("child".into()));
    assert_eq!(parent.work_status().state, WorkRunState::Completed);
    assert_eq!(
        parent.work_result().state(),
        PromiseState::Fulfilled(Value::Keyword("parent".into()))
    );
}

#[test]
fn deadlines_are_inherited_and_cancel_at_cooperative_safe_points() {
    let host = WorkHost::new();
    let child_source = Promise::new();
    let task_child_source = child_source.clone();
    let parent = host
        .submit_scoped(
            WorkOptions {
                id: Some(WorkId::new("deadline-parent").unwrap()),
                timeout: Some(Duration::from_millis(5)),
                ..WorkOptions::default()
            },
            move |context| {
                context.submit_child(
                    WorkOptions {
                        id: Some(WorkId::new("deadline-child").unwrap()),
                        timeout: Some(Duration::from_secs(1)),
                        ..WorkOptions::default()
                    },
                    move |_| Ok(Value::Promise(task_child_source)),
                )?;
                Ok(Value::Promise(Promise::new()))
            },
        )
        .unwrap();
    host.run(&parent.work_id());
    host.run_next();
    let child = host.resolve("deadline-child").unwrap();
    assert_eq!(parent.deadline(), child.deadline());

    std::thread::sleep(Duration::from_millis(8));
    parent.work_status();
    assert_eq!(parent.work_status().state, WorkRunState::Cancelled);
    assert_eq!(child.work_status().state, WorkRunState::Cancelled);
}

#[test]
fn detached_children_outlive_parent_cancellation() {
    let host = WorkHost::new();
    let parent = host
        .submit_scoped(
            WorkOptions::with_id("detach-parent").unwrap(),
            move |context| {
                context.submit_child(
                    WorkOptions {
                        id: Some(WorkId::new("detached-child").unwrap()),
                        detached: true,
                        ..WorkOptions::default()
                    },
                    move |_| Ok(Value::Number(42)),
                )?;
                Ok(Value::Promise(Promise::new()))
            },
        )
        .unwrap();
    host.run(&parent.work_id());
    let child = host.resolve("detached-child").unwrap();
    assert!(child.work_status().detached);
    assert_eq!(child.work_status().parent_id, None);

    assert!(parent.cancel(Value::Keyword("stop-parent".into())));
    assert_eq!(child.work_status().state, WorkRunState::Queued);
    host.run(&child.work_id());
    assert_eq!(child.work_status().state, WorkRunState::Completed);
}
