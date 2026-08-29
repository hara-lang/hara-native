package hara.truffle;

import org.junit.runner.RunWith;

/** Executes the protocol-backed Lang compiler and registry code.test suites. */
@RunWith(HaraJUnitRunner.class)
@HaraTestSource({
  "lib/test-lang/lang/protocol_test.hal",
  "lib/test-lang/lang/common/compiler_test.hal",
  "lib/test-lang/lang/common/grammar_api_test.hal",
  "lib/test-lang/lang/common/emit_test.hal"
})
public final class HaraLangCompilerTest {}
