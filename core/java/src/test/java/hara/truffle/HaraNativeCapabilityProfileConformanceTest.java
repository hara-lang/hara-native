package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.lang.base.Ex;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import org.junit.Test;

/** Verifies the shared native-host capability matrix without ambient host authority. */
public class HaraNativeCapabilityProfileConformanceTest {
  private static final Path PROFILES =
      Path.of("rust/assets/native-capability-profiles-v1.edn");

  @Test
  public void sharedCapabilityProfilesMatchTheJvmNativeBoundary() throws Exception {
    ProfileCorpus corpus = readProfiles();
    assertEquals(
        List.of("kernel", "sandbox", "file", "network", "native-runtime", "host-call"),
        corpus.capabilities());
    assertEquals(
        List.of("zero", "kernel-sandbox", "file", "network", "native-runtime", "host-call", "all"),
        corpus.profiles().stream().map(Profile::id).toList());

    for (Profile profile : corpus.profiles()) {
      HaraNativeCapabilityBoundary boundary = boundary(profile.grants());
      Set<String> observed = new LinkedHashSet<>();
      for (String capability : corpus.capabilities()) {
        if (boundary.granted(capability)) observed.add(capability);
      }
      assertEquals(profile.id(), profile.grants(), observed);

      for (String capability : corpus.capabilities()) {
        if (profile.grants().contains(capability)) {
          boundary.require("Profile", "probe", capability);
        } else {
          Ex.Info denial =
              assertThrows(
                  Ex.Info.class,
                  () -> boundary.require("Profile", "probe", capability));
          assertEquals(
              "std.native.Profile/probe requires capability :" + capability, denial.getMessage());
        }
      }
    }
  }

  private static HaraNativeCapabilityBoundary boundary(Set<String> grants) {
    return new HaraNativeCapabilityBoundary(
        grants.contains("kernel"),
        grants.contains("sandbox"),
        grants.contains("file"),
        grants.contains("network"),
        grants.contains("native-runtime"),
        grants.contains("host-call"));
  }

  private static ProfileCorpus readProfiles() throws Exception {
    Object value = Parser.LispReader.readString(Files.readString(PROFILES), null);
    assertTrue(value instanceof IMapType);
    IMapType root = (IMapType) value;
    assertEquals("hara.native/capability-profiles/v1", root.lookup(Keyword.create("format")));
    List<String> capabilities = keywords(root.lookup(Keyword.create("capabilities")), "capabilities");
    assertEquals(capabilities.size(), new LinkedHashSet<>(capabilities).size());

    Object profilesValue = root.lookup(Keyword.create("profiles"));
    assertTrue(profilesValue instanceof ILinearType);
    ILinearType profiles = (ILinearType) profilesValue;
    List<Profile> result = new ArrayList<>();
    for (int index = 0; index < profiles.count(); index++) {
      Object profileValue = profiles.nth(index);
      assertTrue(profileValue instanceof IMapType);
      IMapType profile = (IMapType) profileValue;
      Object id = profile.lookup(Keyword.create("id"));
      assertTrue(id instanceof Keyword);
      Set<String> grants = new LinkedHashSet<>(keywords(profile.lookup(Keyword.create("grants")), "grants"));
      assertTrue(capabilities.containsAll(grants));
      result.add(new Profile(((Keyword) id).getName(), grants));
    }
    assertFalse(result.isEmpty());
    assertNotNull(result.stream().filter(profile -> profile.id().equals("zero")).findFirst().orElse(null));
    return new ProfileCorpus(List.copyOf(capabilities), List.copyOf(result));
  }

  private static List<String> keywords(Object value, String field) {
    assertTrue(field + " must be a vector", value instanceof ILinearType);
    ILinearType values = (ILinearType) value;
    List<String> result = new ArrayList<>();
    for (int index = 0; index < values.count(); index++) {
      Object entry = values.nth(index);
      assertTrue(field + " entries must be keywords", entry instanceof Keyword);
      result.add(((Keyword) entry).getName());
    }
    return List.copyOf(result);
  }

  private record Profile(String id, Set<String> grants) {}

  private record ProfileCorpus(List<String> capabilities, List<Profile> profiles) {}
}
