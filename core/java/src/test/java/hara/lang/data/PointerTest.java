package hara.lang.data;

import hara.lang.protocol.Constant;
import hara.lang.protocol.IContext;
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
    IContext runtime = args -> args.length;

    assertEquals(5, pointer.applyIn(runtime, new Object[] {1, 2, 3}));
    assertEquals(4, pointer.invokeIn(runtime, 1, 2));
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
}
