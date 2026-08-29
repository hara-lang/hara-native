package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.interop.ArityException;
import com.oracle.truffle.api.interop.InteropLibrary;
import com.oracle.truffle.api.interop.TruffleObject;
import com.oracle.truffle.api.library.ExportLibrary;
import com.oracle.truffle.api.library.ExportMessage;
import hara.lang.base.Eq;
import java.util.ArrayList;
import java.util.List;

/** A Clojure-shaped multimethod: dispatches on the result of a function call. */
@ExportLibrary(InteropLibrary.class)
public final class HaraMultiFunction implements TruffleObject {
  private final HaraContext context;
  private final Object dispatchFunction;
  private final List<Method> methods = new ArrayList<>();
  private Object defaultMethod;

  public HaraMultiFunction(HaraContext context, Object dispatchFunction) {
    this.context = context;
    this.dispatchFunction = dispatchFunction;
  }

  @TruffleBoundary
  public void addMethod(Object dispatchValue, Object method) {
    if (dispatchValue instanceof hara.lang.data.Keyword
        && ((hara.lang.data.Keyword) dispatchValue).getNamespace() == null
        && ((hara.lang.data.Keyword) dispatchValue).getName().equals("default")) {
      defaultMethod = method;
      return;
    }
    for (int i = 0; i < methods.size(); i++) {
      if (Eq.eq(dispatchValue, methods.get(i).dispatchValue)) {
        methods.set(i, new Method(dispatchValue, method));
        return;
      }
    }
    methods.add(new Method(dispatchValue, method));
  }

  @TruffleBoundary
  public Object invoke(Object[] arguments) {
    Object dispatchValue = context.invokeCallable(dispatchFunction, arguments);
    Object selected = null;
    for (Method method : methods) {
      if (Eq.eq(dispatchValue, method.dispatchValue)) {
        selected = method.function;
        break;
      }
    }
    if (selected == null) selected = defaultMethod;
    if (selected == null) {
      throw new HaraException("No multimethod method for dispatch value " + dispatchValue);
    }
    return context.invokeCallable(selected, arguments);
  }

  @ExportMessage
  boolean isExecutable() {
    return true;
  }

  @ExportMessage
  Object execute(Object[] arguments) throws ArityException {
    return HaraBox.export(invoke(arguments));
  }

  @Override
  public String toString() {
    return "#<multifn>";
  }

  private static final class Method {
    private final Object dispatchValue;
    private final Object function;

    private Method(Object dispatchValue, Object function) {
      this.dispatchValue = dispatchValue;
      this.function = function;
    }
  }
}
