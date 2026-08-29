package hara.truffle;

import java.util.Set;

/**
 * Trusted entry point implemented by a prebuilt provider JAR.
 *
 * <p>The runtime constructs implementations through an isolated package class loader only after the
 * package resolver has selected the JVM variant and the loader has verified its artifact digest.
 * Ordinary Hara code never receives this object or the registration surface.
 */
public interface JvmPackageProvider extends AutoCloseable {
  String ABI = "hara.provider.jvm.v1";

  /** Stable package/provider identity, normally the package owner/name coordinate. */
  String identity();

  /** ABI implemented by this provider. */
  default String abi() {
    return ABI;
  }

  /** Capabilities the provider intends to publish into this runtime. */
  default Set<String> capabilities() {
    return Set.of();
  }

  /** Publish factories through the restricted host-owned registration surface. */
  void register(Registration registration) throws Exception;

  @Override
  default void close() throws Exception {}

  interface Registration {
    void filesystem(IFilesystemFactory factory);
  }
}
