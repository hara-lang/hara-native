package hara.truffle;

/** Installs the private runtime provider beneath the public tool.vm facade. */
public final class ToolVmLibraryProvider implements HaraLibraryProvider {
  @Override
  public String namespace() {
    return "tool.vm.provider";
  }

  @Override
  public void install(HaraContext context) {
    ToolVmLibrary.install(context, namespace());
  }
}
