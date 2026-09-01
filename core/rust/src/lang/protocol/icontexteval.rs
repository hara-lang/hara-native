use hara_protocol_macros::hara_protocol;

/// Runtime-owned pointer evaluation. `eval_ptr` is the asynchronous boundary;
/// `eval_await_ptr` and `invoke_ptr` are synchronous result boundaries.
#[hara_protocol(namespace = "std.protocol.icontexteval", name = "IContextEval")]
pub trait IContextEval<P, A, O, V, T> {
    type Async;

    #[hara_method(value = "evaluate", arity = 3)]
    fn evaluate(&mut self, request: V, options: O) -> Self::Async;
    #[hara_method(value = "evaluate-raw", arity = 3)]
    fn evaluate_raw(&mut self, request: V, options: O) -> Self::Async;
    #[hara_method(value = "eval-ptr", arity = 4)]
    fn eval_ptr(&mut self, pointer: &P, arguments: A, options: O) -> Self::Async;
    #[hara_method(value = "eval-await-ptr", arity = 4)]
    fn eval_await_ptr(&mut self, pointer: &P, arguments: A, options: O) -> V;
    #[hara_method(value = "tags-ptr", arity = 2)]
    fn tags_ptr(&self, pointer: &P) -> T;
    #[hara_method(value = "deref-ptr", arity = 2)]
    fn deref_ptr(&mut self, pointer: &P) -> V;
    #[hara_method(value = "display-ptr", arity = 2)]
    fn display_ptr(&self, pointer: &P) -> V;
    #[hara_method(value = "invoke-ptr", arity = 3)]
    fn invoke_ptr(&mut self, pointer: &P, arguments: A) -> V;
    #[hara_method(value = "transform-in-ptr", arity = 3)]
    fn transform_in_ptr(&self, pointer: &P, arguments: A) -> A;
    #[hara_method(value = "transform-out-ptr", arity = 3)]
    fn transform_out_ptr(&self, pointer: &P, value: V) -> V;
}
