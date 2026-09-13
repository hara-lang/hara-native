package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.icomponent", name = "IComponent")
public interface IComponent {

  @HaraMethod(value = "props", arity = 1)
  IMetadata getProps();

  @HaraMethod(value = "status", arity = 1)
  IMetadata getStatus();

  /** Implementations may interpret level and return their own query value. */
  @HaraMethod(value = "info", arity = 2)
  default Object info(Object level) {
    return getStatus();
  }

  /** A health result may be structured data, not only a boolean. */
  @HaraMethod(value = "health", arity = 1)
  default Object health() {
    return isStarted();
  }

  @HaraMethod(value = "started?", arity = 1)
  boolean isStarted();

  @HaraMethod(value = "stopped?", arity = 1)
  boolean isStopped();

  @HaraMethod(value = "start", arity = 1)
  IComponent start();

  @HaraMethod(value = "stop", arity = 1)
  IComponent stop();

  @HaraMethod(value = "kill", arity = 1)
  default IComponent kill() {
    return this.stop();
  }

  @HaraMethod(value = "remote?", arity = 1)
  default boolean isRemote() {
    return false;
  }
}
