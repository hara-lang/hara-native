#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub(crate) fn time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(error) => -i64::try_from(error.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub(crate) fn time_ns() -> i64 {
    use std::sync::OnceLock;
    use std::time::Instant;

    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    i64::try_from(ORIGIN.get_or_init(Instant::now).elapsed().as_nanos()).unwrap_or(i64::MAX)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "raw-wasm"))]
#[link(wasm_import_module = "env")]
extern "C" {
    fn hara_time_ms() -> i64;
    fn hara_time_ns() -> i64;
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "raw-wasm"))]
pub(crate) fn time_ms() -> i64 {
    unsafe { hara_time_ms() }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "raw-wasm"))]
pub(crate) fn time_ns() -> i64 {
    unsafe { hara_time_ns() }
}

#[cfg(all(
    target_arch = "wasm32",
    target_os = "unknown",
    not(feature = "raw-wasm")
))]
pub(crate) fn time_ms() -> i64 {
    js_sys::Date::now() as i64
}

#[cfg(all(
    target_arch = "wasm32",
    target_os = "unknown",
    not(feature = "raw-wasm")
))]
pub(crate) fn time_ns() -> i64 {
    use wasm_bindgen::{JsCast, JsValue};

    let global = js_sys::global();
    let performance = js_sys::Reflect::get(&global, &JsValue::from_str("performance"))
        .expect("browser performance clock is unavailable");
    let now = js_sys::Reflect::get(&performance, &JsValue::from_str("now"))
        .expect("browser performance.now is unavailable")
        .dyn_into::<js_sys::Function>()
        .expect("browser performance.now is not callable");
    let milliseconds = now
        .call0(&performance)
        .expect("browser performance.now failed")
        .as_f64()
        .expect("browser performance.now returned a non-number");
    (milliseconds * 1_000_000.0) as i64
}

#[cfg(all(test, any(not(target_arch = "wasm32"), target_os = "wasi")))]
mod tests {
    #[test]
    fn wall_clock_is_epoch_milliseconds() {
        assert!(super::time_ms() > 1_000_000_000_000);
    }

    #[test]
    fn monotonic_clock_is_runtime_local_and_nondecreasing() {
        let before = super::time_ns();
        let after = super::time_ns();
        assert!(before >= 0);
        assert!(after >= before);
    }
}
