package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.icontextlifecycle", name = "IContextLifeCycle")
public interface IContextLifeCycle {
  @HaraMethod(value = "has-module?", arity = 2)
  boolean hasModule(Object moduleId);

  @HaraMethod(value = "setup-module", arity = 2)
  void setupModule(Object moduleId);

  @HaraMethod(value = "teardown-module", arity = 2)
  void teardownModule(Object moduleId);

  @HaraMethod(value = "has-setup-ptr?", arity = 2)
  boolean hasSetupPtr(IPointer pointer);

  @HaraMethod(value = "setup-ptr", arity = 2)
  void setupPtr(IPointer pointer);

  @HaraMethod(value = "teardown-ptr", arity = 2)
  void teardownPtr(IPointer pointer);
}
