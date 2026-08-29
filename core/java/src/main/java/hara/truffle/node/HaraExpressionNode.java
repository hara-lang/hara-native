package hara.truffle.node;

import com.oracle.truffle.api.frame.VirtualFrame;
import com.oracle.truffle.api.instrumentation.GenerateWrapper;
import com.oracle.truffle.api.instrumentation.InstrumentableNode;
import com.oracle.truffle.api.instrumentation.StandardTags;
import com.oracle.truffle.api.instrumentation.Tag;
import com.oracle.truffle.api.instrumentation.ProbeNode;
import com.oracle.truffle.api.nodes.Node;
import com.oracle.truffle.api.source.SourceSection;

@GenerateWrapper
public abstract class HaraExpressionNode extends Node implements InstrumentableNode {
  private SourceSection sourceSection;

  public abstract Object execute(VirtualFrame frame);

  @Override
  public boolean isInstrumentable() {
    return true;
  }

  @Override
  public WrapperNode createWrapper(ProbeNode probeNode) {
    return new HaraExpressionNodeWrapper(this, probeNode);
  }

  @Override
  public boolean hasTag(Class<? extends Tag> tag) {
    return tag == StandardTags.ExpressionTag.class
        || (tag == StandardTags.CallTag.class && this instanceof HaraNodes.Invoke)
        || (tag == StandardTags.WriteVariableTag.class
            && (this instanceof HaraNodes.SetVar || this instanceof HaraNodes.SetField));
  }

  public void setHaraSourceSection(SourceSection sourceSection) {
    this.sourceSection = sourceSection;
  }

  @Override
  public SourceSection getSourceSection() {
    return sourceSection;
  }
}
