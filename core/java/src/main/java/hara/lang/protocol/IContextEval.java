package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Runtime-owned pointer evaluation, transformation, dereferencing, and display. */
@HaraProtocolBinding(namespace = "std.protocol.icontexteval", name = "IContextEval")
public interface IContextEval {
  @HaraMethod(value = "evaluate", arity = 3)
  Object evaluate(Object request, Object options);

  @HaraMethod(value = "evaluate-raw", arity = 3)
  Object evaluateRaw(Object request, Object options);

  @HaraMethod(value = "eval-ptr", arity = 4)
  Object evalPtr(IPointer pointer, Object arguments, Object options);

  @HaraMethod(value = "eval-await-ptr", arity = 4)
  Object evalAwaitPtr(IPointer pointer, Object arguments, Object options);

  @HaraMethod(value = "tags-ptr", arity = 2)
  Object tagsPtr(IPointer pointer);

  @HaraMethod(value = "deref-ptr", arity = 2)
  Object derefPtr(IPointer pointer);

  @HaraMethod(value = "display-ptr", arity = 2)
  Object displayPtr(IPointer pointer);

  @HaraMethod(value = "invoke-ptr", arity = 3)
  Object invokePtr(IPointer pointer, Object arguments);

  @HaraMethod(value = "transform-in-ptr", arity = 3)
  Object transformInPtr(IPointer pointer, Object arguments);

  @HaraMethod(value = "transform-out-ptr", arity = 3)
  Object transformOutPtr(IPointer pointer, Object value);
}
