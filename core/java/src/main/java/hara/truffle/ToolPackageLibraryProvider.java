package hara.truffle;

/** Installs the private runtime provider beneath the public tool.package facade. */
public final class ToolPackageLibraryProvider implements HaraLibraryProvider {
  @Override public String namespace() { return "tool.package.provider"; }
  @Override public void install(HaraContext context) {
    ToolPackageLibrary.install(context, namespace());
  }
}
