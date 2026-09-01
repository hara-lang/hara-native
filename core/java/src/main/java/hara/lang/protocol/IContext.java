package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Callable root context with pointer ownership checks. */
@HaraProtocolBinding(namespace = "std.protocol.icontext", name = "IContext")
public interface IContext {
  @HaraMethod(value = "call", arity = -1, variadic = true)
  Object call(Object... args);

  @HaraMethod(value = "has-ptr?", arity = 2)
  default boolean hasPtr(IPointer pointer) {
    return false;
  }
}
