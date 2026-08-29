package hara.truffle;

import com.oracle.truffle.api.source.Source;
import hara.kernel.builtin.BuiltinUtil;

/** Internal source/form execution boundary owned by one {@link HaraContext}. */
final class Evaluator {
  @FunctionalInterface
  interface SourceExecutor {
    Object execute(Source source);
  }

  private final SourceExecutor executor;

  Evaluator(SourceExecutor executor) {
    this.executor = executor;
  }

  Object evalSource(String sourceText, String name) {
    try {
      Source source = Source.newBuilder(HaraLanguage.ID, sourceText, name).build();
      return executor.execute(source);
    } catch (RuntimeException error) {
      if (error instanceof HaraException) {
        throw error;
      }
      throw new HaraException("Unable to evaluate Hara source " + name + ": " + error.getMessage());
    }
  }

  Object evalForm(Object form, String name) {
    return evalSource(BuiltinUtil.prStr(HaraBox.unwrap(form)), name);
  }
}
