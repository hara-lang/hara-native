package hara.truffle;

import static hara.truffle.GitHubFilesystemFixture.join;
import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.nio.charset.StandardCharsets;
import java.util.List;
import org.junit.Test;

public class GitHubFilesystemTest {
  @Test
  public void immutableCommitMountReadsAndEnumeratesWithoutFollowingLinks() {
    try (GitHubFilesystemFixture fixture = new GitHubFilesystemFixture()) {
      IFilesystem filesystem = fixture.open("read-only", fixture.client.initialCommit());
      IFilesystem.Descriptor descriptor = filesystem.descriptor();
      assertEquals("github", descriptor.kind());
      assertTrue(descriptor.readOnly());
      assertTrue(descriptor.capabilities().contains(IFilesystem.Capability.READ));
      assertFalse(descriptor.capabilities().contains(IFilesystem.Capability.WRITE));
      assertFalse(descriptor.display().contains("credential"));

      IFilesystem.Entry readme = join(filesystem.stat(context(), "/README.md"));
      assertEquals(IFilesystem.EntryType.FILE, readme.type());
      assertNull(readme.modifiedAt());
      assertEquals(fixture.client.readmeBlob(), readme.id());
      assertArrayEquals("hello".getBytes(StandardCharsets.UTF_8), read(filesystem, "/README.md"));

      assertEquals(
          IFilesystem.EntryType.SYMLINK,
          join(filesystem.stat(context(), "/link")).type());
      assertFailure("unsupported", () -> read(filesystem, "/link"));
      assertEquals(
          IFilesystem.EntryType.OTHER,
          join(filesystem.stat(context(), "/vendor")).type());

      IFilesystem.EntryPage first =
          join(filesystem.entriesPage(context(), "/", new IFilesystem.PageRequest(null, 2)));
      assertEquals(List.of("/README.md", "/link"), paths(first.entries()));
      assertTrue(first.nextToken() != null && !first.nextToken().isBlank());
      IFilesystem.EntryPage second =
          join(
              filesystem.entriesPage(
                  context(), "/", new IFilesystem.PageRequest(first.nextToken(), 2)));
      assertEquals(List.of("/src", "/vendor"), paths(second.entries()));
      assertNull(second.nextToken());

      assertFailure(
          "permission-denied",
          () ->
              join(
                  filesystem.write(
                      context(),
                      "/new.bin",
                      new byte[] {1},
                      new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, false),
                      IFilesystem.MutationContext.none())));
    }
  }

  @Test
  public void writableBranchCommitsBytesAndRejectsStaleEntryRevisions() {
    try (GitHubFilesystemFixture fixture = new GitHubFilesystemFixture()) {
      IFilesystem first = fixture.open("commit", "heads/main");
      IFilesystem second = fixture.open("commit", "heads/main");
      String initialHead = fixture.client.head();
      IFilesystem.Entry stale = join(first.stat(context(), "/README.md"));

      IFilesystem.Mutation created =
          join(
              first.write(
                  context(),
                  "/data/new.bin",
                  new byte[] {0, 1, 0, (byte) 255},
                  new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, true),
                  IFilesystem.MutationContext.none()));
      assertNotEquals(initialHead, created.mountRevision());
      assertEquals(fixture.client.head(), created.mountRevision());
      assertArrayEquals(new byte[] {0, 1, 0, (byte) 255}, read(first, "/data/new.bin"));
      assertEquals(1, fixture.client.commitMessages().size());

      IFilesystem.Mutation replaced =
          join(
              first.write(
                  context(),
                  "/README.md",
                  "changed".getBytes(StandardCharsets.UTF_8),
                  new IFilesystem.WriteOptions(IFilesystem.WriteMode.REPLACE, false),
                  new IFilesystem.MutationContext(stale.revision(), null)));
      assertEquals(fixture.client.head(), replaced.mountRevision());
      assertEquals(2, fixture.client.commitMessages().size());

      assertFailure(
          "conflict",
          () ->
              join(
                  second.write(
                      context(),
                      "/README.md",
                      "stale".getBytes(StandardCharsets.UTF_8),
                      new IFilesystem.WriteOptions(IFilesystem.WriteMode.REPLACE, false),
                      new IFilesystem.MutationContext(stale.revision(), null))));
      assertEquals(2, fixture.client.commitMessages().size());
    }
  }

  @Test
  public void nonForcedRefMovementRejectsWithoutOverwritingTheNewHead() {
    try (GitHubFilesystemFixture fixture = new GitHubFilesystemFixture()) {
      IFilesystem filesystem = fixture.open("commit", "heads/main");
      fixture.client.moveBeforeNextUpdate();
      String competingHead = fixture.client.competingHead();
      assertFailure(
          "conflict",
          () ->
              join(
                  filesystem.write(
                      context(),
                      "/raced.bin",
                      new byte[] {7},
                      new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, false),
                      IFilesystem.MutationContext.none())));
      assertEquals(competingHead, fixture.client.head());
      assertFailure("not-found", () -> join(filesystem.stat(context(), "/raced.bin")));
    }
  }

  @Test
  public void copyMoveDeleteAndDirectoryPoliciesRemainExplicit() {
    try (GitHubFilesystemFixture fixture = new GitHubFilesystemFixture()) {
      IFilesystem filesystem = fixture.open("commit", "heads/main");
      join(
          filesystem.copy(
              context(),
              "/README.md",
              "/copy.md",
              new IFilesystem.CopyOptions(false, false, false),
              IFilesystem.MutationContext.none()));
      assertArrayEquals("hello".getBytes(StandardCharsets.UTF_8), read(filesystem, "/copy.md"));

      join(
          filesystem.move(
              context(),
              "/copy.md",
              "/moved.md",
              new IFilesystem.MoveOptions(false, false, false),
              IFilesystem.MutationContext.none()));
      assertFailure("not-found", () -> join(filesystem.stat(context(), "/copy.md")));
      join(
          filesystem.delete(
              context(),
              "/moved.md",
              new IFilesystem.DeleteOptions(false),
              IFilesystem.MutationContext.none()));
      assertFailure("not-found", () -> join(filesystem.stat(context(), "/moved.md")));

      join(
          filesystem.copy(
              context(),
              "/src",
              "/source-copy",
              new IFilesystem.CopyOptions(false, false, false),
              IFilesystem.MutationContext.none()));
      assertArrayEquals("(+ 1 2)".getBytes(StandardCharsets.UTF_8), read(filesystem, "/source-copy/main.hal"));
      join(
          filesystem.move(
              context(),
              "/source-copy",
              "/source-moved",
              new IFilesystem.MoveOptions(false, false, false),
              IFilesystem.MutationContext.none()));
      assertArrayEquals("(+ 1 2)".getBytes(StandardCharsets.UTF_8), read(filesystem, "/source-moved/main.hal"));
      assertFailure(
          "directory-not-empty",
          () ->
              join(
                  filesystem.delete(
                      context(),
                      "/source-moved",
                      new IFilesystem.DeleteOptions(false),
                      IFilesystem.MutationContext.none())));

      assertFailure(
          "unsupported",
          () ->
              join(
                  filesystem.mkdir(
                      context(),
                      "/empty",
                      new IFilesystem.MkdirOptions(false, false),
                      IFilesystem.MutationContext.none())));
      assertFailure(
          "unsupported",
          () ->
              join(
                  filesystem.copy(
                      context(),
                      "/README.md",
                      "/preserved.md",
                      new IFilesystem.CopyOptions(false, false, true),
                      IFilesystem.MutationContext.none())));
      assertFailure(
          "unsupported",
          () ->
              join(
                  filesystem.move(
                      context(),
                      "/README.md",
                      "/atomic.md",
                      new IFilesystem.MoveOptions(false, false, true),
                      IFilesystem.MutationContext.none())));
    }
  }

  @Test
  public void cancellationAndCloseRejectReuse() {
    try (GitHubFilesystemFixture fixture = new GitHubFilesystemFixture()) {
      IFilesystem filesystem = fixture.open("commit", "heads/main");
      IFilesystem.CallContext cancelled = context();
      assertTrue(cancelled.cancel());
      assertFailure("cancelled", () -> join(filesystem.stat(cancelled, "/")));
      join(filesystem.close(context()));
      join(filesystem.close(context()));
      assertFailure("provider-closed", () -> join(filesystem.stat(context(), "/")));
    }
  }

  private static byte[] read(IFilesystem filesystem, String path) {
    return join(filesystem.read(context(), path));
  }

  private static IFilesystem.CallContext context() {
    return IFilesystem.CallContext.create();
  }

  private static List<String> paths(List<IFilesystem.Entry> entries) {
    return entries.stream().map(IFilesystem.Entry::path).toList();
  }

  private static void assertFailure(String code, Runnable operation) {
    FilesystemException error = assertThrows(FilesystemException.class, operation::run);
    assertEquals(code, error.code());
    assertEquals("github", error.provider());
  }
}
