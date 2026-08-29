package hara.truffle;

import hara.lang.data.Keyword;
import java.math.BigInteger;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Arrays;
import java.util.HexFormat;
import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import org.bouncycastle.asn1.x9.X9ECParameters;
import org.bouncycastle.crypto.agreement.ECDHBasicAgreement;
import org.bouncycastle.crypto.digests.SHA256Digest;
import org.bouncycastle.crypto.ec.CustomNamedCurves;
import org.bouncycastle.crypto.params.ECDomainParameters;
import org.bouncycastle.crypto.params.ECPrivateKeyParameters;
import org.bouncycastle.crypto.params.ECPublicKeyParameters;
import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters;
import org.bouncycastle.crypto.params.Ed25519PublicKeyParameters;
import org.bouncycastle.crypto.params.X25519PrivateKeyParameters;
import org.bouncycastle.crypto.params.X25519PublicKeyParameters;
import org.bouncycastle.crypto.signers.ECDSASigner;
import org.bouncycastle.crypto.signers.Ed25519Signer;
import org.bouncycastle.crypto.signers.HMacDSAKCalculator;

/** Portable cryptographic primitives exposed by the native Crypto static object. */
public final class NativeCrypto {
  private static final SecureRandom RANDOM = new SecureRandom();
  private static final int MAX_RANDOM_BYTES = 1_048_576;
  private static final X9ECParameters P256 = CustomNamedCurves.getByName("secp256r1");
  private static final ECDomainParameters P256_DOMAIN =
      new ECDomainParameters(P256.getCurve(), P256.getG(), P256.getN(), P256.getH());

  private NativeCrypto() {}

  static void install(HaraContext context, String namespace) {
    HaraNativeLibrary.function(context, namespace, "sha512", NativeCrypto::sha512, "", "[bytes]");
    HaraNativeLibrary.function(context, namespace, "hmac-sha256", NativeCrypto::hmacSha256, "", "[key bytes]");
    HaraNativeLibrary.function(context, namespace, "hmac-sha512", NativeCrypto::hmacSha512, "", "[key bytes]");
    HaraNativeLibrary.function(context, namespace, "random-bytes", NativeCrypto::randomBytes, "", "[size]");
    HaraNativeLibrary.function(context, namespace, "secure-equal?", NativeCrypto::secureEqual, "", "[left right]");
    HaraNativeLibrary.function(context, namespace, "ed25519-keypair", NativeCrypto::ed25519Keypair, "", "[]");
    HaraNativeLibrary.function(context, namespace, "ed25519-public", NativeCrypto::ed25519Public, "", "[private-seed]");
    HaraNativeLibrary.function(context, namespace, "ed25519-sign", NativeCrypto::ed25519Sign, "", "[private-seed message]");
    HaraNativeLibrary.function(context, namespace, "ed25519-verify", NativeCrypto::ed25519Verify, "", "[public-key message signature]");
    HaraNativeLibrary.function(context, namespace, "x25519-keypair", NativeCrypto::x25519Keypair, "", "[]");
    HaraNativeLibrary.function(context, namespace, "x25519-public", NativeCrypto::x25519Public, "", "[private-key]");
    HaraNativeLibrary.function(context, namespace, "x25519-shared", NativeCrypto::x25519Shared, "", "[private-key peer-public-key]");
    HaraNativeLibrary.function(context, namespace, "p256-keypair", NativeCrypto::p256Keypair, "", "[]");
    HaraNativeLibrary.function(context, namespace, "p256-public", NativeCrypto::p256Public, "", "[private-key]");
    HaraNativeLibrary.function(context, namespace, "p256-sign", NativeCrypto::p256Sign, "", "[private-key message]");
    HaraNativeLibrary.function(context, namespace, "p256-verify", NativeCrypto::p256Verify, "", "[public-key message signature]");
    HaraNativeLibrary.function(context, namespace, "p256-shared", NativeCrypto::p256Shared, "", "[private-key peer-public-key]");
  }

  public static Object sha512(HaraContext context, Object[] values) {
    requireArity("sha512", values, 1);
    return digestHex("SHA-512", bytes(values[0], "sha512"));
  }

  public static Object hmacSha256(HaraContext context, Object[] values) {
    return hmac("hmac-sha256", "HmacSHA256", values);
  }

  public static Object hmacSha512(HaraContext context, Object[] values) {
    return hmac("hmac-sha512", "HmacSHA512", values);
  }

  public static Object randomBytes(HaraContext context, Object[] values) {
    requireArity("random-bytes", values, 1);
    Object raw = HaraBox.unwrap(values[0]);
    if (!(raw instanceof Number)) throw error("random-bytes", "expects an integer size");
    long requested = ((Number) raw).longValue();
    if (requested < 0 || requested > MAX_RANDOM_BYTES) {
      throw error("random-bytes", "size must be between 0 and " + MAX_RANDOM_BYTES);
    }
    byte[] output = new byte[(int) requested];
    RANDOM.nextBytes(output);
    return output;
  }

  public static Object secureEqual(HaraContext context, Object[] values) {
    requireArity("secure-equal?", values, 2);
    return MessageDigest.isEqual(
        bytes(values[0], "secure-equal?"), bytes(values[1], "secure-equal?"));
  }

  public static Object ed25519Keypair(HaraContext context, Object[] values) {
    requireArity("ed25519-keypair", values, 0);
    byte[] seed = random(Ed25519PrivateKeyParameters.KEY_SIZE);
    return keypair(seed, ed25519Public(seed));
  }

  public static Object ed25519Public(HaraContext context, Object[] values) {
    requireArity("ed25519-public", values, 1);
    return ed25519Public(fixedBytes(values[0], "ed25519-public", 32));
  }

  public static Object ed25519Sign(HaraContext context, Object[] values) {
    requireArity("ed25519-sign", values, 2);
    byte[] seed = fixedBytes(values[0], "ed25519-sign", 32);
    byte[] message = bytes(values[1], "ed25519-sign");
    Ed25519Signer signer = new Ed25519Signer();
    signer.init(true, new Ed25519PrivateKeyParameters(seed, 0));
    signer.update(message, 0, message.length);
    return signer.generateSignature();
  }

  public static Object ed25519Verify(HaraContext context, Object[] values) {
    requireArity("ed25519-verify", values, 3);
    byte[] publicKey = fixedBytes(values[0], "ed25519-verify", 32);
    byte[] message = bytes(values[1], "ed25519-verify");
    byte[] signature = bytes(values[2], "ed25519-verify");
    if (signature.length != Ed25519PrivateKeyParameters.SIGNATURE_SIZE) return false;
    try {
      Ed25519Signer verifier = new Ed25519Signer();
      verifier.init(false, new Ed25519PublicKeyParameters(publicKey, 0));
      verifier.update(message, 0, message.length);
      return verifier.verifySignature(signature);
    } catch (RuntimeException invalid) {
      return false;
    }
  }

  public static Object x25519Keypair(HaraContext context, Object[] values) {
    requireArity("x25519-keypair", values, 0);
    byte[] privateKey = random(X25519PrivateKeyParameters.KEY_SIZE);
    return keypair(privateKey, x25519Public(privateKey));
  }

  public static Object x25519Public(HaraContext context, Object[] values) {
    requireArity("x25519-public", values, 1);
    return x25519Public(fixedBytes(values[0], "x25519-public", 32));
  }

  public static Object x25519Shared(HaraContext context, Object[] values) {
    requireArity("x25519-shared", values, 2);
    X25519PrivateKeyParameters privateKey =
        new X25519PrivateKeyParameters(fixedBytes(values[0], "x25519-shared", 32), 0);
    X25519PublicKeyParameters publicKey =
        new X25519PublicKeyParameters(fixedBytes(values[1], "x25519-shared", 32), 0);
    byte[] output = new byte[X25519PrivateKeyParameters.SECRET_SIZE];
    try {
      privateKey.generateSecret(publicKey, output, 0);
    } catch (IllegalStateException invalid) {
      throw error("x25519-shared", "peer public key produces an invalid shared secret");
    }
    return output;
  }

  public static Object p256Keypair(HaraContext context, Object[] values) {
    requireArity("p256-keypair", values, 0);
    byte[] privateKey;
    do {
      privateKey = random(32);
    } while (!validP256Scalar(privateKey));
    return keypair(privateKey, p256Public(privateKey));
  }

  public static Object p256Public(HaraContext context, Object[] values) {
    requireArity("p256-public", values, 1);
    return p256Public(p256Private(values[0], "p256-public"));
  }

  public static Object p256Sign(HaraContext context, Object[] values) {
    requireArity("p256-sign", values, 2);
    byte[] privateKey = p256Private(values[0], "p256-sign");
    byte[] digest = digest("SHA-256", bytes(values[1], "p256-sign"));
    ECDSASigner signer = new ECDSASigner(new HMacDSAKCalculator(new SHA256Digest()));
    signer.init(true, new ECPrivateKeyParameters(unsigned(privateKey), P256_DOMAIN));
    BigInteger[] signature = signer.generateSignature(digest);
    BigInteger s = signature[1];
    BigInteger halfOrder = P256.getN().shiftRight(1);
    if (s.compareTo(halfOrder) > 0) s = P256.getN().subtract(s);
    byte[] output = new byte[64];
    fixedUnsigned(signature[0], output, 0);
    fixedUnsigned(s, output, 32);
    return output;
  }

  public static Object p256Verify(HaraContext context, Object[] values) {
    requireArity("p256-verify", values, 3);
    ECPublicKeyParameters publicKey = p256PublicParameter(values[0], "p256-verify");
    byte[] digest = digest("SHA-256", bytes(values[1], "p256-verify"));
    byte[] signature = bytes(values[2], "p256-verify");
    if (signature.length != 64) return false;
    BigInteger r = unsigned(Arrays.copyOfRange(signature, 0, 32));
    BigInteger s = unsigned(Arrays.copyOfRange(signature, 32, 64));
    if (r.signum() <= 0 || s.signum() <= 0 || r.compareTo(P256.getN()) >= 0
        || s.compareTo(P256.getN()) >= 0) return false;
    ECDSASigner verifier = new ECDSASigner();
    verifier.init(false, publicKey);
    return verifier.verifySignature(digest, r, s);
  }

  public static Object p256Shared(HaraContext context, Object[] values) {
    requireArity("p256-shared", values, 2);
    ECPrivateKeyParameters privateKey =
        new ECPrivateKeyParameters(unsigned(p256Private(values[0], "p256-shared")), P256_DOMAIN);
    ECDHBasicAgreement agreement = new ECDHBasicAgreement();
    agreement.init(privateKey);
    BigInteger shared = agreement.calculateAgreement(p256PublicParameter(values[1], "p256-shared"));
    byte[] output = new byte[32];
    fixedUnsigned(shared, output, 0);
    return output;
  }

  private static Object hmac(String operation, String algorithm, Object[] values) {
    requireArity(operation, values, 2);
    byte[] key = bytes(values[0], operation);
    byte[] message = bytes(values[1], operation);
    try {
      Mac mac = Mac.getInstance(algorithm);
      mac.init(new SecretKeySpec(key, algorithm));
      return HexFormat.of().formatHex(mac.doFinal(message));
    } catch (java.security.GeneralSecurityException unavailable) {
      throw error(operation, algorithm + " is unavailable");
    }
  }

  private static byte[] digest(String algorithm, byte[] value) {
    try {
      return MessageDigest.getInstance(algorithm).digest(value);
    } catch (java.security.NoSuchAlgorithmException unavailable) {
      throw error(algorithm.toLowerCase(), algorithm + " is unavailable");
    }
  }

  private static String digestHex(String algorithm, byte[] value) {
    return HexFormat.of().formatHex(digest(algorithm, value));
  }

  private static byte[] random(int size) {
    byte[] output = new byte[size];
    RANDOM.nextBytes(output);
    return output;
  }

  private static Object keypair(byte[] privateKey, byte[] publicKey) {
    return hara.lang.data.Map.Standard.from(
        null,
        Keyword.create("private"), privateKey,
        Keyword.create("public"), publicKey);
  }

  private static byte[] ed25519Public(byte[] seed) {
    return new Ed25519PrivateKeyParameters(seed, 0).generatePublicKey().getEncoded();
  }

  private static byte[] x25519Public(byte[] privateKey) {
    return new X25519PrivateKeyParameters(privateKey, 0).generatePublicKey().getEncoded();
  }

  private static byte[] p256Public(byte[] privateKey) {
    return P256.getG().multiply(unsigned(privateKey)).normalize().getEncoded(true);
  }

  private static ECPublicKeyParameters p256PublicParameter(Object value, String operation) {
    byte[] encoded = bytes(value, operation);
    if (encoded.length != 33) throw error(operation, "expects a compressed 33-byte public key");
    try {
      return new ECPublicKeyParameters(P256.getCurve().decodePoint(encoded), P256_DOMAIN);
    } catch (RuntimeException invalid) {
      throw error(operation, "expects a valid P-256 public key");
    }
  }

  private static byte[] p256Private(Object value, String operation) {
    byte[] privateKey = fixedBytes(value, operation, 32);
    if (!validP256Scalar(privateKey)) throw error(operation, "expects a valid P-256 private key");
    return privateKey;
  }

  private static boolean validP256Scalar(byte[] value) {
    BigInteger scalar = unsigned(value);
    return scalar.signum() > 0 && scalar.compareTo(P256.getN()) < 0;
  }

  private static BigInteger unsigned(byte[] value) {
    return new BigInteger(1, value);
  }

  private static void fixedUnsigned(BigInteger value, byte[] output, int offset) {
    byte[] encoded = value.toByteArray();
    int sourceOffset = encoded.length > 32 ? encoded.length - 32 : 0;
    int length = Math.min(encoded.length, 32);
    System.arraycopy(encoded, sourceOffset, output, offset + 32 - length, length);
  }

  private static byte[] bytes(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof byte[])) throw error(operation, "expects bytes");
    return (byte[]) raw;
  }

  private static byte[] fixedBytes(Object value, String operation, int size) {
    byte[] bytes = bytes(value, operation);
    if (bytes.length != size) throw error(operation, "expects " + size + " bytes");
    return bytes;
  }

  private static void requireArity(String operation, Object[] values, int expected) {
    if (values.length != expected) {
      throw error(operation, "expects " + expected + (expected == 1 ? " argument" : " arguments"));
    }
  }

  private static HaraException error(String operation, String message) {
    return new HaraException("crypto/" + operation + ": " + message);
  }
}
