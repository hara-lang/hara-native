package hara.truffle;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

final class HaraExtensionTestProject {
  private static final Pattern NAMESPACE = Pattern.compile(":namespace\\s+\"([^\"]+)\"\\s*");
  private static final Pattern IDENTITY = Pattern.compile(":identity\\s+\"([^\"]+)\"\\s*");
  private static final Pattern VERSION = Pattern.compile(":version\\s+\"([^\"]+)\"\\s*");

  private HaraExtensionTestProject() {}

  static void write(Path root, String manifest) throws Exception {
    String namespace = required(NAMESPACE, manifest, "namespace");
    String version = required(VERSION, manifest, "version");
    Matcher identity = IDENTITY.matcher(manifest);
    String projectId = identity.find() ? identity.group(1) : "test/" + namespace;
    String declaration = NAMESPACE.matcher(manifest).replaceFirst("");
    declaration = IDENTITY.matcher(declaration).replaceFirst("");
    declaration = VERSION.matcher(declaration).replaceFirst("");
    declaration = declaration.substring(1, declaration.length() - 1);
    Files.writeString(
        root.resolve("project.edn"),
        "{:hara/type :project :hara/version \"1.0.0\" :project/id " + projectId
            + " :project/version \"" + version + "\" :project/source-paths []"
            + " :project/test-paths [] :project/extension-paths [] :project/capabilities #{}"
            + " :project/extensions {" + namespace + " {" + declaration + "}}}");
  }

  private static String required(Pattern pattern, String source, String field) {
    Matcher matcher = pattern.matcher(source);
    if (!matcher.find()) throw new IllegalArgumentException("missing " + field);
    return matcher.group(1);
  }
}
