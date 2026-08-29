package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.junit.Test;

public class HaraSubstrateProtocolTest {
  @Test
  public void substrateCapabilitiesLoadAndDispatchThroughExtendedTypes() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(require 'std.substrate.protocol) "
                      + "(defstruct Fixture [id]) "
                      + "(extend-type Fixture std.substrate.protocol/ISubstrateService "
                      + "  (get-service [node service-id] 42) "
                      + "  (set-service [node service-id service] service) "
                      + "  (remove-service [node service-id] service-id)) "
                      + "(std.substrate.protocol/get-service (Fixture \"fixture-1\") \"answer\")")
              .asLong());
    }
  }

  @Test
  public void missingSubstrateCapabilityImplementationReportsProtocolError() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      PolyglotException error =
          org.junit.Assert.assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(require 'std.substrate.protocol) "
                          + "(defstruct Incomplete []) "
                          + "(std.substrate.protocol/get-service "
                          + "(Incomplete) \"cache\")"));
      assertTrue(error.getMessage().contains("ISubstrateService/get-service"));
    }
  }

  @Test
  public void protocolSurfaceHalFixturePasses() throws Exception {
    String source = Files.readString(Path.of("lib/test/std/substrate/protocol_test.hal"));
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      String result = context.eval(HaraLanguage.ID, source).toString();
      assertTrue(result, !result.contains(":pass false"));
    }
  }

  @Test
  public void atomBackedSubstrateRunsWithoutStudioOrBrowserState() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(require 'std.substrate) "
                      + "(def node (std.substrate/node-create \"node-1\")) "
                      + "(std.substrate.protocol/set-service node \"cache\" 42) "
                      + "(std.substrate.protocol/get-service node \"cache\")")
              .asLong());
    }
  }

  @Test
  public void sharedProtocolConformanceFixtureRuns() throws Exception {
    String source = Files.readString(Path.of("lib/test-fixtures/std/substrate/protocol_conformance.hal"));
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals("[40 42]", context.eval(HaraLanguage.ID, source).toString());
    }
  }

  @Test
  public void sharedSubstrateFrameConformanceFixtureRuns() throws Exception {
    String source = Files.readString(Path.of("lib/test-fixtures/std/substrate/frame_conformance.hal"));
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[\"substrate.v1\" \"request\" \"req-1\" \"client/a\" \"server/b\" \"workspace/main\" {\"trace\" \"trace-1\"} \"math/add\" [19 23] nil nil nil nil nil nil]",
          context.eval(HaraLanguage.ID, source).toString());
      try {
        context.eval(
            HaraLanguage.ID,
            "(do (require 'std.substrate.json) "
                + "(std.substrate.json/decode-frame {:kind :unknown :id \"evt-1\"}))");
        fail("expected invalid substrate frames to throw");
      } catch (PolyglotException expected) {
        // The Hara thrown value is intentionally opaque to the host.
      }
      try {
        context.eval(
            HaraLanguage.ID,
            "(do (require 'std.substrate.json) "
                + "(std.substrate.json/decode-frame \"{bad\"))");
        fail("expected malformed substrate JSON to throw");
      } catch (PolyglotException expected) {
        // The strict JSON reader reports malformed wire input through Hara.
      }
    }
  }

  @Test
  public void sharedSubstrateNodeLifecycleFixtureRuns() throws Exception {
    String source = Files.readString(Path.of("lib/test-fixtures/std/substrate/node_lifecycle_conformance.hal"));
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals("[84 42 :rejected]", context.eval(HaraLanguage.ID, source).toString());
    }
  }

  @Test
  public void substrateNodeHalFixturePasses() throws Exception {
    String source = Files.readString(Path.of("lib/test/std/substrate_test.hal"));
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      String result = context.eval(HaraLanguage.ID, source).toString();
      assertTrue(result, !result.contains(":pass false"));
    }
  }

  @Test
  public void substrateRoutesStreamsAndSettlesTransportRequests() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          84,
          context
              .eval(
                  HaraLanguage.ID,
                  "(require 'std.substrate) "
                      + "(def node (std.substrate/node-create \"node-1\")) "
                      + "(std.substrate.protocol/attach-transport node \"peer-a\" "
                      + "  (fn [frame] "
                      + "    (std.substrate.protocol/receive-frame node "
                      + "      (std.substrate/node-frame :response \"res-1\" \"main\" {} nil [] "
                      + "        (std.substrate/frame-id frame) :ok 84 nil nil nil) "
                      + "      {:transport-id \"peer-a\"}))) "
                      + "(def reply (std.substrate.protocol/request node \"main\" \"sum\" [] "
                      + "  {:id \"req-1\" :transport-id \"peer-a\"})) "
                      + "(promise/value reply)")
              .asLong());
    }
  }

  @Test
  public void substrateCancellationSettlesThePendingPromise() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          ":rejected",
          context
              .eval(
                  HaraLanguage.ID,
                  "(require 'std.substrate) "
                      + "(def node (std.substrate/node-create \"node-1\")) "
                      + "(std.substrate.protocol/attach-transport node \"peer-a\" (fn [frame] nil)) "
                      + "(def pending (std.substrate.protocol/request node \"main\" \"wait\" [] "
                      + "  {:id \"req-cancel\" :transport-id \"peer-a\"})) "
                      + "(std.substrate.protocol/cancel-request node \"req-cancel\" :cancelled) "
                      + "(promise/state pending)")
              .toString());
    }
  }

  @Test
  public void missingLocalRequestHandlerFailsLikeXTalk() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      org.junit.Assert.assertThrows(
          PolyglotException.class,
          () ->
              context.eval(
                  HaraLanguage.ID,
                  "(require 'std.substrate) "
                      + "(def node (std.substrate/node-create \"node-1\")) "
                      + "(std.substrate.protocol/request node \"main\" \"missing\" [] {})"));
    }
  }
}
