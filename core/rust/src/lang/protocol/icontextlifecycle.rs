use hara_protocol_macros::hara_protocol;

#[hara_protocol(
    namespace = "std.protocol.icontextlifecycle",
    name = "IContextLifeCycle"
)]
pub trait IContextLifeCycle<M, P> {
    #[hara_method(value = "has-module?", arity = 2)]
    fn has_module(&self, module: &M) -> bool;
    #[hara_method(value = "setup-module", arity = 2)]
    fn setup_module(&mut self, module: M);
    #[hara_method(value = "teardown-module", arity = 2)]
    fn teardown_module(&mut self, module: &M);
    #[hara_method(value = "has-setup-ptr?", arity = 2)]
    fn has_setup_ptr(&self, pointer: &P) -> bool;
    #[hara_method(value = "setup-ptr", arity = 2)]
    fn setup_ptr(&mut self, pointer: P);
    #[hara_method(value = "teardown-ptr", arity = 2)]
    fn teardown_ptr(&mut self, pointer: &P);
}
