package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.interop.InteropLibrary;
import com.oracle.truffle.api.interop.TruffleObject;
import com.oracle.truffle.api.interop.UnsupportedMessageException;
import com.oracle.truffle.api.library.ExportLibrary;
import com.oracle.truffle.api.library.ExportMessage;
import hara.lang.data.Map;
import hara.lang.declaration.HaraAvailability;
import hara.lang.protocol.IDisplay;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.INamespaced;
import hara.lang.protocol.IObjType;
import hara.lang.protocol.Constant;
import java.util.List;
import java.util.Objects;

/** Canonical, non-callable descriptor for a runtime-owned std.native type. */
@ExportLibrary(InteropLibrary.class)
public final class HaraNativeType
    implements TruffleObject, IDisplay, INamespaced, IObjType {
  private final String namespace;
  private final String name;
  private final List<String> methods;
  private final HaraAvailability availability;
  private final String capability;
  private final IMetadata metadata;

  HaraNativeType(String namespace, String name, List<String> methods) {
    this(namespace, name, methods, HaraAvailability.PORTABLE, "", Map.Standard.EMPTY);
  }

  HaraNativeType(
      String namespace,
      String name,
      List<String> methods,
      HaraAvailability availability,
      String capability) {
    this(namespace, name, methods, availability, capability, Map.Standard.EMPTY);
  }

  private HaraNativeType(
      String namespace,
      String name,
      List<String> methods,
      HaraAvailability availability,
      String capability,
      IMetadata metadata) {
    this.namespace = Objects.requireNonNull(namespace);
    this.name = Objects.requireNonNull(name);
    this.methods = List.copyOf(methods);
    this.availability = Objects.requireNonNull(availability);
    this.capability = Objects.requireNonNull(capability);
    this.metadata = metadata == null ? Map.Standard.EMPTY : metadata;
  }

  public List<String> methods() {
    return methods;
  }

  public HaraAvailability availability() {
    return availability;
  }

  public String capability() {
    return capability;
  }

  @Override
  public String getName() {
    return name;
  }

  @Override
  public String getNamespace() {
    return namespace;
  }

  @Override
  public IMetadata meta() {
    return metadata;
  }

  @Override
  public HaraNativeType withMeta(IMetadata metadata) {
    return new HaraNativeType(namespace, name, methods, availability, capability, metadata);
  }

  @Override
  public long hashCalc(Constant.HashType type) {
    return Objects.hash(namespace, name);
  }

  @Override
  public String display() {
    return "#<native-type " + namespace + "." + name + ">";
  }

  @ExportMessage
  boolean isExecutable() {
    return false;
  }

  @ExportMessage
  Object execute(Object[] arguments) throws UnsupportedMessageException {
    throw UnsupportedMessageException.create();
  }

  @ExportMessage
  @TruffleBoundary
  Object toDisplayString(boolean allowSideEffects) {
    return display();
  }

  @Override
  public boolean equals(Object other) {
    return other instanceof HaraNativeType type
        && namespace.equals(type.namespace)
        && name.equals(type.name);
  }

  @Override
  public int hashCode() {
    return Objects.hash(namespace, name);
  }

  @Override
  public String toString() {
    return display();
  }
}
