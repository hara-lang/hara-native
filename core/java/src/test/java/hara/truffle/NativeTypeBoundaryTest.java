package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;

import hara.lang.protocol.IStreamDuplex;
import org.graalvm.polyglot.Context;
import org.junit.Test;

/** Locks the boundary between public native descriptors and internal runtime values. */
public class NativeTypeBoundaryTest {
  @Test
  public void runtimeOnlyNamesAreNotNativeDescriptors() {
    for (String name : new String[] {"Instrumentation", "Env", "Duplex", "Builtins", "Seq"}) {
      assertFalse(
          "Unexpected native descriptor: " + name,
          HaraNativeDeclarations.METHODS.containsKey(name));
      assertFalse(
          "Unexpected native annotation: " + name,
          HaraNativeDeclarations.bindings().stream()
              .anyMatch(binding -> binding.name().equals(name)));
    }

    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "[true true true true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(nil? (resolve 'std.native.Instrumentation)) "
                      + "(nil? (resolve 'std.native.Env)) "
                      + "(nil? (resolve 'std.native.Duplex)) "
                      + "(nil? (resolve 'std.native.Builtins)) "
                      + "(nil? (resolve 'std.native.Seq))]")
              .toString());
    }
  }

  @Test
  public void duplexUsesTheAnnotatedStreamProtocolBoundary() {
    assertEquals(
        IStreamDuplex.class, HaraProtocolDeclarations.discover().get("IStreamDuplex"));
  }

  @Test
  public void seqRemainsAnInternalLazyValue() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "[true true 1 2]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [value (seq [1 2])] "
                      + "[(seq? value) (seq? (rest [1 2])) (first value) (first (rest value))])")
              .toString());
    }
  }

  @Test
  public void byteArraysRetainTheExistingMutableNativeBoundary() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "[:std.native.ByteBuffer true 9]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [value (Bytes/new 1 2)] "
                      + "[(type value) (bytes? value) "
                      + "(do (Bytes/set value 0 9) (Bytes/get value 0))])")
              .toString());
    }
  }

  @Test
  public void baseDeclarationsUseExplicitNamespaceHandles() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "[42 \"hello Ada\" \"session-token\" :std.native.Nil]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [target (std.native.Base/namespace 'example.native) "
                      + "user-type (std.native.Base/struct target 'User (std.native.Base/vector 'name)) "
                      + "session-type (std.native.Base/mutable target 'Session (std.native.Base/vector 'token)) "
                      + "_ (std.native.Base/def target 'answer (fn [] 42) nil) "
                      + "greeting (std.native.Base/protocol target 'IGreeting {'greet 1} (std.native.Base/vector)) "
                      + "_ (std.native.Base/extend target user-type greeting {'greet (fn [user] \"hello Ada\")}) "
                      + "user ((std.protocol.ideref.IDeref/deref (std.native.Base/resolve target '->User)) \"Ada\") "
                      + "session ((std.protocol.ideref.IDeref/deref (std.native.Base/resolve target '->Session)) \"session-token\")] "
                      + "[((std.protocol.ideref.IDeref/deref (std.native.Base/resolve target 'answer))) "
                      + "((std.protocol.ideref.IDeref/deref (std.native.Base/resolve target 'greet)) user) "
                      + "(std.native.Base/field session :token) "
                      + "(do (std.native.Base/protocol target 'IGreeting {'welcome 1} (std.native.Base/vector)) "
                      + "(type (std.native.Base/resolve target 'greet)))])")
              .toString());
    }
  }

  @Test
  public void baseDefRegistersMacrosInTheExplicitTargetNamespace() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      context.eval(
          HaraLanguage.ID,
          "(let [target (std.native.Base/namespace 'example.macro)] "
              + "(std.native.Base/def target 'identity-form "
              + "(fn [form environment value] value) {:macro true}))");
      assertEquals(
          "42", context.eval(HaraLanguage.ID, "(example.macro/identity-form 42)").toString());
    }
  }

}
