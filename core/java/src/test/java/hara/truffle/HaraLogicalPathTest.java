package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertThrows;

import java.util.List;
import org.junit.Test;

public class HaraLogicalPathTest {
  @Test
  public void normalisesPortableLogicalPaths() {
    assertEquals("/", HaraLogicalPath.normalise(""));
    assertEquals("/src/main.hal", HaraLogicalPath.normalise("src//./lib/../main.hal"));
    assertEquals("/src/test.hal", HaraLogicalPath.join("/src", "/test.hal"));
    assertEquals("/test.hal", HaraLogicalPath.resolve("/src", "/test.hal"));
    assertEquals("/src", HaraLogicalPath.parent("/src/main.hal"));
    assertNull(HaraLogicalPath.parent("/"));
    assertEquals("", HaraLogicalPath.fileName("/"));
    assertEquals(List.of("src", "main.hal"), HaraLogicalPath.segments("/src/main.hal"));
  }

  @Test
  public void rejectsHostSyntaxAndMountEscape() {
    HaraLogicalPath.Error escape =
        assertThrows(HaraLogicalPath.Error.class, () -> HaraLogicalPath.normalise("../../secret"));
    assertEquals("outside-root", escape.code());
    assertEquals(
        "invalid-path",
        assertThrows(
                HaraLogicalPath.Error.class,
                () -> HaraLogicalPath.normalise("C:/windows/system.ini"))
            .code());
    assertEquals(
        "invalid-path",
        assertThrows(
                HaraLogicalPath.Error.class, () -> HaraLogicalPath.normalise("src\\main.hal"))
            .code());
  }
}
