package hara.lang.context;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Pointer;
import hara.lang.protocol.IComponent;
import hara.lang.protocol.IContext;
import hara.lang.protocol.IMetadata;
import hara.lang.resource.ResourceMode;
import hara.lang.resource.ResourceRegistry;
import hara.lang.resource.ResourceSpec;
import java.util.Map;
import org.junit.Test;

public class ContextRuntimeTest {
  @Test
  public void spacesResolveConfigurationAndManageResourceLifecycle() {
    CountingRuntime runtime = new CountingRuntime();
    ContextRegistry contexts = new ContextRegistry();
    contexts.install(
        "test",
        Map.of(
            "context",
            "test",
            "scratch",
            runtime,
            "config",
            Map.of("nested", Map.of("default", 1)),
            "rt",
            Map.of(
                "default",
                Map.of(
                    "key",
                    "default",
                    "resource",
                    "test.runtime",
                    "config",
                    Map.of("nested", Map.of("runtime", 2))))));
    ResourceRegistry resources = new ResourceRegistry();
    resources.add(
        new ResourceSpec(
            "test.runtime", ResourceMode.NAMESPACE, Map.of(), options -> runtime));
    Space space = new Space("testing", contexts, resources);

    space.contextSet("test", "default", Map.of("nested", Map.of("caller", 3)));
    @SuppressWarnings("unchecked")
    Map<String, Object> descriptor = (Map<String, Object>) space.contextGet("test");
    @SuppressWarnings("unchecked")
    Map<String, Object> nested = (Map<String, Object>) descriptor.get("config");
    @SuppressWarnings("unchecked")
    Map<String, Object> merged = (Map<String, Object>) nested.get("nested");
    assertEquals(Map.of("default", 1, "runtime", 2, "caller", 3), merged);
    assertSame(runtime, space.runtimeCurrent("test"));
    assertFalse(space.runtimeStarted("test"));

    assertSame(runtime, space.runtimeStart("test"));
    assertTrue(space.runtimeStarted("test"));
    assertEquals(1, runtime.starts);
    assertSame(runtime, space.runtimeStart("test"));
    assertEquals(1, runtime.starts);

    space.runtimeStop("test");
    assertTrue(space.runtimeStopped("test"));
    assertEquals(1, runtime.stops);
  }

  @Test
  public void pointersAreStructuralDescriptorsAndUseExplicitRuntimes() {
    CountingRuntime runtime = new CountingRuntime();
    Pointer pointer =
        new Pointer(
            hara.lang.data.Keyword.create("test"),
            Map.of(hara.lang.data.Keyword.create("value"), 42));
    assertEquals(
        new Pointer(
            hara.lang.data.Keyword.create("test"),
            Map.of(hara.lang.data.Keyword.create("value"), 42)),
        pointer);
    assertEquals(42, pointer.lookup(hara.lang.data.Keyword.create("value")));
    assertEquals(1, pointer.count());
    assertEquals(5, pointer.applyIn(runtime, new Object[] {1, 2, 3}));
  }

  private static final class CountingRuntime implements IContext, IComponent {
    int starts;
    int stops;
    boolean started;

    @Override
    public Object call(Object... args) {
      return args.length;
    }

    @Override
    @SuppressWarnings("unchecked")
    public Object derefPtr(hara.lang.protocol.IPointer pointer) {
      return ((hara.lang.protocol.ILookup<Object, Object>) pointer).lookup("value");
    }

    @Override
    public IMetadata getProps() {
      return null;
    }

    @Override
    public IMetadata getStatus() {
      return null;
    }

    @Override
    public boolean isStarted() {
      return started;
    }

    @Override
    public boolean isStopped() {
      return !started;
    }

    @Override
    public IComponent start() {
      starts++;
      started = true;
      return this;
    }

    @Override
    public IComponent stop() {
      stops++;
      started = false;
      return this;
    }
  }
}
