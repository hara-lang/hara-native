package hara.lang.data;

import hara.lang.protocol.Constant;
import hara.lang.protocol.IContext;
import hara.lang.protocol.IContextEval;
import hara.lang.protocol.IPointer;
import java.util.LinkedHashMap;
import java.util.Map;
import org.junit.Test;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertThrows;

public class PointerTest {

  @Test
  public void pointerIsAContextQualifiedStructuralDescriptor() {
    Map<Object, Object> fields = new LinkedHashMap<>();
    fields.put(Keyword.create("id"), "ROOT");
    fields.put(Keyword.create("value"), 42);
    Pointer pointer = new Pointer(Keyword.create("kernel"), fields);

    fields.put(Keyword.create("value"), 0);
    assertEquals(Keyword.create("kernel"), pointer.ptrContext());
    assertEquals("ROOT", pointer.lookup(Keyword.create("id")));
    assertEquals(42, pointer.lookup(Keyword.create("value")));
    assertEquals(2L, pointer.count());
    assertEquals(
        new Pointer(
            Keyword.create("kernel"),
            Map.of(Keyword.create("id"), "ROOT", Keyword.create("value"), 42)),
        pointer);
    assertEquals(
        "#ptr {:context :kernel :id \"ROOT\" :value 42}", pointer.display());
    assertEquals(Constant.ObjType.POINTER, pointer.getObjType());
    assertEquals("POINTER", pointer.getObjName());
  }

  @Test
  public void pointerUsesAnExplicitContextRuntime() {
    Pointer pointer = new Pointer(Keyword.create("test"), Map.of(Keyword.create("value"), 42));
    TestRuntime runtime = new TestRuntime();

    assertEquals(3, pointer.applyIn(runtime, new Object[] {1, 2, 3}));
    assertThrows(IllegalArgumentException.class, () -> pointer.applyIn(new Object(), new Object[0]));
    assertThrows(IllegalStateException.class, pointer::deref);
    assertThrows(IllegalStateException.class, pointer::applyDefault);
    assertSame(pointer, pointer.withMeta(null));
  }

  @Test
  public void canonicalDescriptorValidationIsStable() {
    Pointer pointer =
        Pointer.fromDescriptor(
            hara.lang.data.Map.Standard.from(
                null, Keyword.create("context"), Keyword.create("test"), Keyword.create("id"), "x"));
    assertEquals(Keyword.create("test"), pointer.context());
    assertEquals("x", pointer.lookup(Keyword.create("id")));
    assertThrows(
        IllegalArgumentException.class,
        () -> Pointer.fromDescriptor(Map.of(Keyword.create("id"), "x")));
    assertThrows(
        IllegalArgumentException.class,
        () ->
            Pointer.fromDescriptor(
                Map.of(Keyword.create("context"), "test", Keyword.create("id"), "x")));
    assertThrows(
        IllegalArgumentException.class,
        () -> new Pointer(Keyword.create("test"), Map.of("id", "x")));
  }

  private static final class TestRuntime implements IContext, IContextEval {
    @Override
    public Object call(Object... args) {
      return args.length;
    }

    @Override
    public Object evaluate(Object request, Object options) {
      return request;
    }

    @Override
    public Object evaluateRaw(Object request, Object options) {
      return request;
    }

    @Override
    public Object evalPtr(IPointer pointer, Object arguments, Object options) {
      return arguments;
    }

    @Override
    public Object evalAwaitPtr(IPointer pointer, Object arguments, Object options) {
      return arguments;
    }

    @Override
    public Object tagsPtr(IPointer pointer) {
      return new Object[0];
    }

    @Override
    public Object derefPtr(IPointer pointer) {
      return pointer;
    }

    @Override
    public Object displayPtr(IPointer pointer) {
      return pointer;
    }

    @Override
    public Object invokePtr(IPointer pointer, Object arguments) {
      return ((Object[]) arguments).length;
    }

    @Override
    public Object transformInPtr(IPointer pointer, Object arguments) {
      return arguments;
    }

    @Override
    public Object transformOutPtr(IPointer pointer, Object value) {
      return value;
    }
  }
}
