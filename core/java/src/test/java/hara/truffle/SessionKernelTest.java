package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.kernel.Conn;
import hara.lang.protocol.IApplicable;
import hara.lang.protocol.IComponent;
import hara.lang.protocol.IContext;
import java.net.Socket;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Set;
import org.junit.Test;

public class SessionKernelTest {
  private static SessionModel.SessionId sessionId(String value) {
    return SessionModel.SessionId.parse(value);
  }

  @Test
  public void localAndRespClientsShareRootAcrossListenerRestarts() throws Exception {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      kernel.root().eval("(def answer 41)");

      try (HaraServer first = new HaraServer(kernel, "127.0.0.1", 0, false)) {
        first.start();
        assertEquals("42", legacyEval(first.port(), "(+ answer 1)"));
      }

      assertEquals("user", kernel.root().currentNamespace());
      kernel.root().eval("(def answer 42)");

      try (HaraServer second = new HaraServer(kernel, "127.0.0.1", 0, false)) {
        second.start();
        assertEquals("42", legacyEval(second.port(), "answer"));
      }
    }
  }

  @Test
  public void sessionsIsolateDefinitionsInsideOneBroker() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session alpha = kernel.create(sessionId("alpha"));
      SessionKernel.Session beta = kernel.create(sessionId("beta"));
      alpha.eval("(def answer 41)");
      beta.eval("(def answer 6)");
      assertEquals("41", alpha.eval("answer").toString());
      assertEquals("6", beta.eval("answer").toString());
    }
  }

  @Test
  public void childSessionsDoNotInheritPrivilegedRootAuthority() {
    try (SessionKernel kernel = new SessionKernel(true, true, true)) {
      SessionKernel.Session root = kernel.root();
      SessionKernel.Session child = kernel.create(sessionId("zero-authority"));

      assertTrue(root.eval("(deref (Host/capability? \"filesystem\"))").asBoolean());
      assertTrue(root.eval("(deref (Host/capability? \"network/socket\"))").asBoolean());
      assertTrue(root.eval("(deref (Host/capability? \"process\"))").asBoolean());

      assertFalse(child.eval("(deref (Host/capability? \"filesystem\"))").asBoolean());
      assertFalse(child.eval("(deref (Host/capability? \"network/socket\"))").asBoolean());
      assertFalse(child.eval("(deref (Host/capability? \"process\"))").asBoolean());

      SessionKernel.SessionAuthorityPolicy policy = child.authority();
      assertFalse(policy.hostFilesystem);
      assertFalse(policy.hostNetwork);
      assertFalse(policy.hostProcess);
      assertFalse(policy.reflection);
      assertFalse(policy.packages);
      assertFalse(policy.project);
      assertEquals("zero", ((SessionModel.SessionStatus) child.getStatus()).authority().profile());
    }
  }

  @Test
  public void sessionsConformToContextComponentAndApplicativeProtocols() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session alpha = kernel.create(sessionId("alpha"));
      SessionKernel.Session beta = kernel.create(sessionId("beta"));

      assertTrue(alpha instanceof IContext);
      assertTrue(alpha instanceof IComponent);
      assertTrue(alpha instanceof IApplicable);
      assertTrue(alpha.isStarted());
      assertEquals("user", ((SessionModel.SessionStatus) alpha.getProps()).namespace());
      assertEquals("zero", ((SessionModel.SessionStatus) alpha.getProps()).authority().profile());

      assertEquals(41L, alpha.call("(do (ns alpha.core) (def answer 41) answer)"));
      assertEquals("alpha.core", alpha.currentNamespace());
      assertEquals("user", beta.currentNamespace());
      assertSame(alpha, alpha.applyDefault());
      assertEquals(42L, alpha.applyIn(beta, new Object[] {"(+ 20 22)"}));
      Object[] arguments = new Object[] {"answer"};
      assertSame(arguments, alpha.transformIn(beta, arguments));
      assertEquals(41L, alpha.transformOut(beta, arguments, 41L));

      alpha.stop();
      assertTrue(alpha.isStopped());
      assertFalse(alpha.isStarted());
      assertThrows(IllegalStateException.class, () -> alpha.call("answer"));
      assertThrows(IllegalStateException.class, alpha::start);
    }
  }

  @Test
  public void filesystemAttachmentConfinesFilesAndResetsSessionState() throws Exception {
    Path root = Files.createTempDirectory("hara-session-files");
    try (SessionKernel kernel = new SessionKernel(true, false)) {
      SessionKernel.Session session = kernel.create(sessionId("mounted"));
      assertFalse(session.eval("(deref (Host/capability? \"filesystem\"))").asBoolean());
      assertEquals(SessionModel.SessionState.ACTIVE, session.state());
      assertEquals(null, session.filesystemMount());
      assertEquals(null, ((SessionModel.SessionStatus) session.getStatus()).filesystem());
      session.eval("(def stale-value 42)");
      SessionModel.SessionMountId mount = kernel.createFilesystem(root);
      kernel.attachFilesystem(session.id(), mount);
      assertEquals(mount, session.filesystemMount());
      assertEquals(1, kernel.filesystemInfo(mount).attachments());
      assertTrue(session.eval("(deref (Host/capability? \"filesystem\"))").asBoolean());
      assertEquals("zero", session.authority().profile());
      session.eval("(deref (std.native.File/write \"/state.bin\" (bytes 1 2 3)))");
      assertTrue(Files.exists(root.resolve("state.bin")));
      try {
        session.eval("stale-value");
        throw new AssertionError("reattachment must reset namespace state");
      } catch (IllegalArgumentException expected) {
        assertTrue(expected.getMessage().contains("Unbound"));
      }
      kernel.detachFilesystem(session.id());
      assertEquals(null, session.filesystemMount());
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      kernel.closeFilesystem(mount);
    } finally {
      Files.deleteIfExists(root.resolve("state.bin"));
      Files.deleteIfExists(root);
    }
  }

  @Test
  public void kernelOwnsOpaqueFilesystemMountsAndAttachmentCounts() throws Exception {
    Path firstRoot = Files.createTempDirectory("hara-session-first");
    Path secondRoot = Files.createTempDirectory("hara-session-second");
    try (SessionKernel kernel = new SessionKernel(true, false)) {
      SessionKernel.Session session = kernel.create(sessionId("mounted"));
      SessionModel.SessionMountId first = kernel.createFilesystem(firstRoot);
      SessionModel.SessionMountId second = kernel.createFilesystem(secondRoot);

      assertNotEquals(first, second);
      assertThrows(IllegalArgumentException.class, () -> SessionModel.SessionMountId.of(0));
      assertEquals(null, kernel.filesystem(session.id()));
      assertEquals(0, kernel.filesystemInfo(first).attachments());

      kernel.attachFilesystem(session.id(), first);
      kernel.attachFilesystem(session.id(), first);
      assertEquals(first, kernel.filesystem(session.id()));
      assertEquals(1, kernel.filesystemInfo(first).attachments());
      assertThrows(IllegalArgumentException.class, () -> kernel.closeFilesystem(first));

      kernel.attachFilesystem(session.id(), second);
      assertEquals(second, kernel.filesystem(session.id()));
      assertEquals(0, kernel.filesystemInfo(first).attachments());
      assertEquals(1, kernel.filesystemInfo(second).attachments());
      kernel.closeFilesystem(first);

      kernel.closeSession(session.id());
      assertEquals(0, kernel.filesystemInfo(second).attachments());
      kernel.closeFilesystem(second);
    } finally {
      Files.deleteIfExists(firstRoot);
      Files.deleteIfExists(secondRoot);
    }
  }

  @Test
  public void sessionStopReleasesKernelOwnedFilesystemExactlyOnce() throws Exception {
    Path root = Files.createTempDirectory("hara-session-stop");
    try (SessionKernel kernel = new SessionKernel(true, false)) {
      SessionKernel.Session session = kernel.create(sessionId("stopped"));
      SessionModel.SessionMountId mount = kernel.createFilesystem(root);
      kernel.attachFilesystem(session.id(), mount);

      session.stop();
      session.stop();

      assertEquals(SessionModel.SessionState.CLOSED, session.state());
      assertEquals(0, kernel.filesystemInfo(mount).attachments());
      kernel.closeFilesystem(mount);
    } finally {
      Files.deleteIfExists(root);
    }
  }

  @Test
  public void sessionBoundariesValidateIdentityAndExposeExplicitLifecycle() {
    assertThrows(IllegalArgumentException.class, () -> sessionId("bad/name"));
    SessionModel.SessionId id = sessionId("workspace.alpha");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(id);
      assertEquals(id, session.id());
      assertEquals(SessionModel.SessionState.ACTIVE, session.state());
      assertEquals(
          SessionModel.SessionState.ACTIVE,
          ((SessionModel.SessionStatus) session.getStatus()).state());

      kernel.closeSession(id);
      assertEquals(SessionModel.SessionState.CLOSED, session.state());
      assertEquals(null, session.filesystemMount());
      session.close();
      assertEquals(SessionModel.SessionState.CLOSED, session.state());
    }
  }

  @Test
  public void developmentResourcesAndSealedBundlesUseDistinctCatalogs() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      kernel.registerDevelopmentResource("demo/value.hal", "(ns demo.value) (def value 42)");
      assertEquals(Set.of("demo/value.hal"), kernel.developmentResourceNames());

      byte[] sealed = new byte[] {1, 2, 3};
      kernel.registerBundle("sha256:demo", sealed);
      sealed[0] = 9;
      assertTrue(java.util.Arrays.equals(new byte[] {1, 2, 3}, kernel.bundle("sha256:demo")));
      kernel.registerBundle("sha256:demo", new byte[] {1, 2, 3});
      assertThrows(
          IllegalArgumentException.class,
          () -> kernel.registerBundle("sha256:demo", new byte[] {4, 5, 6}));

      assertTrue(kernel.removeDevelopmentResource("demo/value.hal"));
      assertTrue(kernel.developmentResourceNames().isEmpty());
      assertTrue(java.util.Arrays.equals(new byte[] {1, 2, 3}, kernel.bundle("sha256:demo")));
    }
  }

  private static String legacyEval(int port, String source) throws Exception {
    try (Socket socket = new Socket("127.0.0.1", port)) {
      Conn conn = new Conn(socket);
      conn.write("EVAL", "ROOT", source);
      return text(conn.read());
    }
  }

  private static String text(Object value) {
    if (value instanceof byte[])
      return new String((byte[]) value, java.nio.charset.StandardCharsets.UTF_8);
    return String.valueOf(value);
  }
}
