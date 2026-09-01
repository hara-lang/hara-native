package hara.lang.context;

import hara.lang.protocol.IContext;
import hara.lang.protocol.IContextEval;
import hara.lang.protocol.IPointer;

/** Safe fallback context used when no runtime is active. */
public final class NullContext implements IContext, IContextEval {
  public static final NullContext INSTANCE = new NullContext();

  private NullContext() {}

  @Override
  public Object call(Object... args) {
    throw new IllegalStateException("Context runtime is not active");
  }

  @Override
  public Object evaluate(Object request, Object options) {
    throw inactive();
  }

  @Override
  public Object evaluateRaw(Object request, Object options) {
    throw inactive();
  }

  @Override
  public Object evalPtr(IPointer pointer, Object arguments, Object options) {
    throw inactive();
  }

  @Override
  public Object evalAwaitPtr(IPointer pointer, Object arguments, Object options) {
    throw inactive();
  }

  @Override
  public Object tagsPtr(IPointer pointer) {
    throw inactive();
  }

  @Override
  public Object derefPtr(IPointer pointer) {
    throw inactive();
  }

  @Override
  public Object displayPtr(IPointer pointer) {
    throw inactive();
  }

  @Override
  public Object invokePtr(IPointer pointer, Object arguments) {
    throw inactive();
  }

  @Override
  public Object transformInPtr(IPointer pointer, Object arguments) {
    throw inactive();
  }

  @Override
  public Object transformOutPtr(IPointer pointer, Object value) {
    throw inactive();
  }

  private static IllegalStateException inactive() {
    return new IllegalStateException("Context runtime is not active");
  }
}
