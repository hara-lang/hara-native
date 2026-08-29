package hara.truffle;

import com.oracle.truffle.api.interop.TruffleObject;
import hara.lang.base.G;
import hara.lang.protocol.IDeref;
import hara.lang.protocol.IDisplay;
import java.util.Objects;

/** Immutable, normalized portable schema value. Provenance is not structural identity. */
public final class HaraSchemaType implements TruffleObject, IDeref<Object>, IDisplay {
  private final Object form;
  private final HalcSchema.Type ast;
  private final HaraVar origin;

  HaraSchemaType(Object form, HalcSchema.Type ast, HaraVar origin) {
    this.form = form;
    this.ast = Objects.requireNonNull(ast);
    this.origin = origin;
  }

  public Object form() {
    return form;
  }

  public HalcSchema.Type ast() {
    return ast;
  }

  public HaraVar origin() {
    return origin;
  }

  @Override
  public Object deref() {
    return HalcSchema.shorthand(ast);
  }

  @Override
  public String display() {
    return "(schema " + G.display(form) + ")";
  }

  @Override
  public boolean equals(Object other) {
    return other instanceof HaraSchemaType schema && ast.equals(schema.ast);
  }

  @Override
  public int hashCode() {
    return ast.hashCode();
  }

  @Override
  public String toString() {
    return display();
  }
}
