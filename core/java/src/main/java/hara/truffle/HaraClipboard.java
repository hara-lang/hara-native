package hara.truffle;

import java.awt.Toolkit;
import java.awt.datatransfer.Clipboard;
import java.awt.datatransfer.DataFlavor;
import java.awt.datatransfer.StringSelection;

/** Text transport only; permission checks belong to the evaluator boundary. */
final class HaraClipboard {
  static Clipboard system() {
    return Toolkit.getDefaultToolkit().getSystemClipboard();
  }

  static String copy(Clipboard clipboard, String text) {
    clipboard.setContents(new StringSelection(text), null);
    return text;
  }

  static String paste(Clipboard clipboard) {
    try {
      return (String) clipboard.getData(DataFlavor.stringFlavor);
    } catch (java.awt.datatransfer.UnsupportedFlavorException | java.io.IOException error) {
      throw new HaraException("OS/clipboard-paste failed: " + error.getMessage());
    }
  }
}
