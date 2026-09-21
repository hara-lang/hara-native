package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotSame;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import com.oracle.truffle.api.RootCallTarget;
import com.oracle.truffle.api.RootNode;
import com.oracle.truffle.api.Truffle;
import com.oracle.truffle.api.frame.VirtualFrame;
import hara.truffle.bytecode.HbcBytecodeRootNode;
import hara.truffle.bytecode.HbcProgram;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class HaraMetaSpaceTest {
  @Test
  public void linksOneTargetPerProgramIdentityAndFunction() {
    HaraMetaSpace metaSpace = new HaraMetaSpace();
    HbcProgram program = program();
    RootCallTarget first = target();
    RootCallTarget second = target();
    AtomicInteger compilations = new AtomicInteger();

    assertSame(
        first,
        metaSpace.link(
            program,
            0,
            () -> {
              compilations.incrementAndGet();
              return first;
            }));
    assertSame(
        first,
        metaSpace.link(
            program,
            0,
            () -> {
              compilations.incrementAndGet();
              return second;
            }));
    assertEquals(1, compilations.get());
    assertEquals(1, metaSpace.size());

    HbcProgram equivalent = program();
    assertNotSame(program, equivalent);
    assertSame(second, metaSpace.link(equivalent, 0, () -> second));
    assertEquals(2, metaSpace.size());
  }

  @Test
  public void concurrentLinksCompileOnlyOnce() throws Exception {
    HaraMetaSpace metaSpace = new HaraMetaSpace();
    HbcProgram program = program();
    RootCallTarget target = target();
    AtomicInteger compilations = new AtomicInteger();
    CountDownLatch compiling = new CountDownLatch(1);
    CountDownLatch release = new CountDownLatch(1);
    ExecutorService executor = Executors.newFixedThreadPool(4);
    try {
      List<Future<RootCallTarget>> futures = new ArrayList<>();
      for (int index = 0; index < 4; index++) {
        futures.add(
            executor.submit(
                () ->
                    metaSpace.link(
                        program,
                        0,
                        () -> {
                          compilations.incrementAndGet();
                          compiling.countDown();
                          await(release);
                          return target;
                        })));
      }
      if (!compiling.await(5, TimeUnit.SECONDS)) {
        throw new AssertionError("link compiler was not entered");
      }
      release.countDown();
      for (Future<RootCallTarget> future : futures) {
        assertSame(target, future.get());
      }
      assertEquals(1, compilations.get());
    } finally {
      release.countDown();
      executor.shutdownNow();
    }
  }

  @Test
  public void clearDropsLinksAndCloseRejectsNewOnes() {
    HaraMetaSpace metaSpace = new HaraMetaSpace();
    HbcProgram program = program();
    RootCallTarget target = target();

    metaSpace.link(program, 0, () -> target);
    metaSpace.clear();
    assertNull(metaSpace.get(program, 0));
    metaSpace.close();
    metaSpace.close();
    assertEquals(0, metaSpace.size());
    assertThrows(IllegalStateException.class, () -> metaSpace.get(program, 0));
    assertThrows(IllegalStateException.class, () -> metaSpace.link(program, 0, () -> target));
  }

  @Test
  public void repeatedNativeExecutionReusesTheLinkedTarget() {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.eval(HaraLanguage.ID, "nil");
      polyglot.enter();
      try {
        HaraContext context = HaraLanguage.currentContext();
        HbcProgram program = executableProgram();
        RootCallTarget first =
            HbcBytecodeRootNode.compile(HaraLanguage.currentLanguage(), program);
        assertTrue(first.getRootNode() instanceof HbcBytecodeRootNode);
        assertEquals(42L, first.call());

        RootCallTarget second =
            HbcBytecodeRootNode.compile(HaraLanguage.currentLanguage(), program);
        assertSame(first, second);
        assertEquals(42L, second.call());
        assertSame(first, context.metaSpace().get(program, program.entry()));
        assertEquals(1, context.metaSpace().size());
      } finally {
        polyglot.leave();
      }
    }
  }

  @Test
  public void contextFinalizationClearsLinkedTargets() {
    HaraMetaSpace[] metaSpace = new HaraMetaSpace[1];
    HbcProgram[] program = new HbcProgram[1];
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.eval(HaraLanguage.ID, "nil");
      polyglot.enter();
      try {
        HaraContext context = HaraLanguage.currentContext();
        metaSpace[0] = context.metaSpace();
        program[0] = executableProgram();
        assertEquals(
            42L,
            HbcBytecodeRootNode.compile(HaraLanguage.currentLanguage(), program[0]).call());
        assertEquals(1, metaSpace[0].size());
      } finally {
        polyglot.leave();
      }
    }
    assertNotNull(metaSpace[0]);
    assertEquals(0, metaSpace[0].size());
    assertThrows(IllegalStateException.class, () -> metaSpace[0].get(program[0], 0));
  }

  private static HbcProgram executableProgram() {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(null, null),
            List.of());
    return new HbcProgram(
        "metaspace-execution",
        List.of(42L),
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        List.of(entry),
        0);
  }

  private static HbcProgram program() {
    return new HbcProgram(List.of(), List.of(), List.of(), 0);
  }

  private static RootCallTarget target() {
    return Truffle.getRuntime()
        .createCallTarget(
            new RootNode(null) {
              @Override
              public Object execute(VirtualFrame frame) {
                return null;
              }
            });
  }

  private static void await(CountDownLatch latch) {
    try {
      latch.await();
    } catch (InterruptedException interrupted) {
      Thread.currentThread().interrupt();
      throw new AssertionError(interrupted);
    }
  }
}
