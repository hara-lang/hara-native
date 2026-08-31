package hara.kernel;

import hara.kernel.base.Parser;
import hara.kernel.base.RT;
import hara.lang.base.G;
import hara.truffle.HaraPackageTool;

import java.io.IOException;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.PrintStream;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

public class NativeMain {

  public static void main(String[] args) throws IOException {
    NativeMode.enable();
    if (args.length == 0) {
      printUsage();
      return;
    }

    String command = args[0];
    String[] restArgs = java.util.Arrays.copyOfRange(args, 1, args.length);

    if ("help".equals(command) || "--help".equals(command) || "-h".equals(command)) {
      printUsage();
      return;
    }

    if ("version".equals(command) || "--version".equals(command) || "-V".equals(command)) {
      System.out.println("hara-native-jvm 0.1.16");
      return;
    }

    if ("eval".equals(command)) {
      requireArg(command, restArgs, "<code>");
      printResult(evalString(restArgs[0]));
      return;
    }

    if ("stdin".equals(command)) {
      printResult(evalAll(createRuntime(), new InputStreamReader(System.in, StandardCharsets.UTF_8)));
      return;
    }

    if ("run".equals(command)) {
      requireArg(command, restArgs, "<file>");
      printResult(evalFile(Path.of(restArgs[0])));
      return;
    }

    if ("test".equals(command)) {
      requireArg(command, restArgs, "<suite.json> [group ...]");
      int status = runTestSuite(Path.of(restArgs[0]), java.util.Arrays.copyOfRange(restArgs, 1, restArgs.length), System.out, System.err);
      if (status != 0) System.exit(status);
      return;
    }

    if ("bundle".equals(command)) {
      requireArg(command, restArgs, "verify|install|inspect <archive.harp>");
      String operation = restArgs[0];
      if (!"verify".equals(operation) && !"install".equals(operation) && !"inspect".equals(operation)) {
        System.err.println("hara-native bundle supports verify, install, or inspect");
        System.exit(2);
      }
      int status =
          HaraPackageTool.run(
              java.util.Arrays.copyOfRange(restArgs, 0, restArgs.length), System.out, System.err);
      if (status != 0) System.exit(status);
      return;
    }

    if ("repl".equals(command)) {
      runRepl(System.out, System.err);
      return;
    }

    Path implicitFile = Path.of(command);
    if (Files.exists(implicitFile)) {
      printResult(evalFile(implicitFile));
      return;
    }

    System.err.println("Unknown hara-native command: " + command);
    printUsage();
  }

  private static void requireArg(String command, String[] args, String usage) {
    if (args.length == 0) {
      System.err.println("Usage: hara-native " + command + " " + usage);
      System.exit(1);
    }
  }

  private static RT.Instance<Object> createRuntime() {
    Foundation foundation = new Foundation();
    RT.Instance<Object> rt = new RT.Instance<>(foundation, "ROOT");
    foundation.RTS.put(rt._key, rt);
    return rt;
  }

  private static Object evalString(String code) {
    try {
      return evalAll(createRuntime(), new java.io.StringReader(code));
    } catch (IOException impossible) {
      throw new IllegalStateException(impossible);
    }
  }

  private static Object evalFile(Path path) throws IOException {
    try (Reader reader = Files.newBufferedReader(path, StandardCharsets.UTF_8)) {
      return evalAll(createRuntime(), reader);
    }
  }

  private static Object evalAll(RT.Instance<Object> rt, Reader input) throws IOException {
    hara.kernel.base.Reader reader = new hara.kernel.base.Reader(input);
    Object eof = new Object();
    Object result = null;
    while (true) {
      Object form = Parser.LispReader.read(reader, false, eof, false, null);
      if (form == eof) {
        return result;
      }
      result = rt.eval(form);
    }
  }

  private static void runRepl(PrintStream output, PrintStream error) throws IOException {
    RT.Instance<Object> runtime = createRuntime();
    output.println("hara-native-jvm core REPL; Ctrl-D to exit");
    try (BufferedReader input = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))) {
      for (String line; (line = input.readLine()) != null; ) {
        if (line.isBlank()) continue;
        try {
          printResult(evalAll(runtime, new java.io.StringReader(line)));
        } catch (RuntimeException exception) {
          error.println("ERROR " + message(exception));
        }
      }
    }
  }

  static int runTestSuite(Path path, String[] groupArguments, PrintStream output, PrintStream error) {
    try {
      var suite = NativeTestSuite.read(path);
      var cases = NativeTestSuite.select(suite, java.util.List.of(groupArguments));
      RT.Instance<Object> runtime = createRuntime();
      int failures = 0;
      for (NativeTestSuite.Case test : cases) {
        String actual;
        boolean passed;
        try {
          actual = G.display(evalAll(runtime, new java.io.StringReader(test.source())));
          passed = !test.expected().error() && actual.equals(test.expected().value());
          if (!passed) actual = "value " + actual;
        } catch (RuntimeException | IOException exception) {
          actual = "error " + message(exception);
          passed = test.expected().error() && actual.contains(test.expected().value());
        }
        if (passed) {
          output.println("PASS  " + test.group() + "/" + test.id());
        } else {
          String expected = test.expected().error()
              ? "error containing " + test.expected().value()
              : "value " + test.expected().value();
          output.println("FAIL  " + test.group() + "/" + test.id());
          output.println("      expected: " + expected);
          output.println("      actual:   " + actual);
          failures++;
        }
      }
      output.println(
          "SUMMARY selected=" + cases.size() + " passed=" + (cases.size() - failures) + " failed=" + failures);
      return failures == 0 ? 0 : 1;
    } catch (IOException | IllegalArgumentException exception) {
      error.println("hara-native test: " + message(exception));
      return 2;
    }
  }

  private static String message(Exception exception) {
    return exception.getMessage() == null ? exception.getClass().getSimpleName() : exception.getMessage();
  }

  private static void printResult(Object result) {
    if (result != null) {
      System.out.println(G.display(result));
    }
  }

  private static void printUsage() {
    System.out.println("hara-native eval <code>");
    System.out.println("hara-native run <file>");
    System.out.println("hara-native test <suite.json> [group ...]");
    System.out.println("hara-native stdin");
    System.out.println("hara-native bundle verify <archive.harp>");
    System.out.println("hara-native bundle install <archive.harp>");
    System.out.println("hara-native bundle inspect <archive.harp>");
    System.out.println("hara-native help");
    System.out.println("Test suites use hara-native/test-suite/1 JSON and run selected cases serially in one runtime.");
    System.out.println("Hara libraries are loaded from verified packages; this host embeds no HAL source.");
  }
}
