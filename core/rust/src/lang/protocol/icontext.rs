use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.icontext", name = "IContext")]
pub trait IContext<A, P> {
  type Output;

  #[hara_method(value = "call", arity = -1, variadic = true, min_arity = 1)]
  fn call(&mut self, arguments: A) -> Self::Output;

  #[hara_method(value = "has-ptr?", arity = 2)]
  fn has_ptr(&self, pointer: &P) -> bool;
}
