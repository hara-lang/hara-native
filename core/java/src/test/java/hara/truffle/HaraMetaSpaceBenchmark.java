package hara.truffle;

import com.oracle.truffle.api.RootCallTarget;
import hara.truffle.bytecode.HbcBytecodeRootNode;
import hara.truffle.bytecode.HbcProgram;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import java.util.Arrays;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import org.graalvm.polyglot.Context;

/**
 * Manual benchmark for linked HBC targets.
 *
 * <p>Run with {@code java ... HaraMetaSpaceBenchmark [samples warmup iterations]}. The uncached
 * path clears the context-owned metaspace before every link, so the reported ratio isolates cache
 * lookup from native target construction.
 */
public final class HaraMetaSpaceBenchmark {
  private HaraMetaSpaceBenchmark() {}

  public static void main(String[] args) {
    int samples = argument(args, 0, 20);
    int warmup = argument(args, 1, 10);
    int iterations = argument(args, 2, 100);
    if (samples < 1 || warmup < 0 || iterations < 1) {
      throw new IllegalArgumentException("samples and iterations must be positive; warmup must not be negative");
    }

    Stats cached = measureCached(samples, warmup, iterations);
    Stats uncached = measureUncached(samples, warmup, iterations);
    double ratio = (double) uncached.medianNs() / cached.medianNs();
    System.out.printf(
        Locale.ROOT,
        "{\"benchmark\":\"hbc-link-metaspace\",\"samples\":%d,\"warmup\":%d,\"iterations\":%d,"
            + "\"cached_median_ns\":%d,\"cached_p95_ns\":%d,\"uncached_median_ns\":%d,"
            + "\"uncached_p95_ns\":%d,\"uncached_over_cached\":%.3f}%n",
        samples,
        warmup,
        iterations,
        cached.medianNs(),
        cached.p95Ns(),
        uncached.medianNs(),
        uncached.p95Ns(),
        ratio);
  }

  private static Stats measureCached(int samples, int warmup, int iterations) {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "nil");
      context.enter();
      try {
        HaraLanguage language = HaraLanguage.currentLanguage();
        HaraContext haraContext = HaraLanguage.currentContext();
        HbcProgram program = executableProgram();
        RootCallTarget linked = HbcBytecodeRootNode.compile(language, program);
        require(linked != null, "native link returned null");
        require(Long.valueOf(42L).equals(linked.call()), "native link returned the wrong result");
        for (int index = 0; index < warmup; index++) {
          require(
              HbcBytecodeRootNode.compile(language, program) == linked,
              "cached warmup returned a different target");
        }
        long[] timings = new long[samples];
        for (int sample = 0; sample < samples; sample++) {
          long started = System.nanoTime();
          for (int iteration = 0; iteration < iterations; iteration++) {
            require(
                HbcBytecodeRootNode.compile(language, program) == linked,
                "cached link returned a different target");
          }
          timings[sample] = (System.nanoTime() - started) / iterations;
        }
        require(haraContext.metaSpace().size() == 1, "cached link was not retained");
        return stats(timings);
      } finally {
        context.leave();
      }
    }
  }

  private static Stats measureUncached(int samples, int warmup, int iterations) {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "nil");
      context.enter();
      try {
        HaraLanguage language = HaraLanguage.currentLanguage();
        HaraContext haraContext = HaraLanguage.currentContext();
        HbcProgram program = executableProgram();
        RootCallTarget previous = null;
        for (int index = 0; index < warmup; index++) {
          haraContext.metaSpace().clear();
          RootCallTarget linked = HbcBytecodeRootNode.compile(language, program);
          require(linked != previous, "uncached warmup reused a cleared target");
          previous = linked;
        }
        long[] timings = new long[samples];
        for (int sample = 0; sample < samples; sample++) {
          haraContext.metaSpace().clear();
          long started = System.nanoTime();
          for (int iteration = 0; iteration < iterations; iteration++) {
            RootCallTarget linked = HbcBytecodeRootNode.compile(language, program);
            require(linked != previous, "uncached link reused a cleared target");
            previous = linked;
            haraContext.metaSpace().clear();
          }
          timings[sample] = (System.nanoTime() - started) / iterations;
        }
        require(haraContext.metaSpace().size() == 0, "uncached benchmark retained a link");
        return stats(timings);
      } finally {
        context.leave();
      }
    }
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
        "metaspace-benchmark",
        List.of(42L),
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        List.of(entry),
        0);
  }

  private static Stats stats(long[] values) {
    long[] sorted = values.clone();
    Arrays.sort(sorted);
    int p95 = Math.max(0, (int) Math.ceil(sorted.length * 0.95) - 1);
    return new Stats(sorted[sorted.length / 2], sorted[p95]);
  }

  private static int argument(String[] args, int index, int fallback) {
    if (args.length <= index) return fallback;
    if (args.length > 3) throw new IllegalArgumentException("expected [samples warmup iterations]");
    return Integer.parseInt(args[index]);
  }

  private static void require(boolean condition, String message) {
    if (!condition) throw new IllegalStateException(message);
  }

  private record Stats(long medianNs, long p95Ns) {}
}
