package com.ninthfreak.carnyx;

import android.graphics.Bitmap;
import android.graphics.BitmapFactory;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;

/**
 * HTTP and image decoding, both done by the platform.
 *
 * <h2>Why this is Java and not a Rust crate</h2>
 *
 * The logo search needs HTTPS, and HTTPS in Rust means a TLS stack — which on a
 * 32-bit ARM head unit means a C dependency that has to be cross-compiled and
 * verified, plus a bundled root store that goes stale the day it ships.
 * {@code src/logos.rs} recommends {@code ureq + rustls + webpki-roots} and that
 * recommendation is deliberately not taken: this app already dexes Java and
 * binds it over JNI for the tuner and for location, so the marginal cost of one
 * more class is nearly nothing, while the marginal cost of a TLS crate is a new
 * C toolchain risk on the one architecture that matters here.
 *
 * Using {@link HttpURLConnection} also means the SYSTEM trust store, which is
 * the correct one: it is what the rest of the device trusts and it tracks OS
 * updates, where bundled roots do not.
 *
 * The same argument covers decoding. {@link BitmapFactory} reads PNG, JPEG, WebP
 * and GIF, is hardware-accelerated on this class of device, and has been hardened
 * against malformed input for a decade — which matters, because every byte it
 * sees came off an image search.
 *
 * <h2>What is NOT decided here</h2>
 *
 * Nothing. This class fetches bytes and converts pixels. Which URL to fetch, how
 * to read DuckDuckGo's two-step response, which candidate to keep and what to do
 * with the pixels are all Rust's, in {@code src/logos.rs}, where they are tested.
 */
public final class CarnyxNet {

    private CarnyxNet() {}

    /**
     * 8 s to connect, 15 s in total.
     *
     * CarFM sets no timeout at all: its fetch can hang until the platform gives
     * up, which on a head unit with a dead SIM is a spinner the driver cannot
     * cancel. These are the numbers {@code src/logos.rs} asks for.
     */
    private static final int CONNECT_MS = 8000;
    private static final int READ_MS = 15000;

    /** Redirects are followed, but not forever, and never off HTTPS onto HTTP. */
    private static final int MAX_REDIRECTS = 5;

    private static HttpURLConnection open(String url, String userAgent) throws Exception {
        URL u = new URL(url);
        if (!"https".equalsIgnoreCase(u.getProtocol())) {
            // Plain HTTP is refused rather than downgraded silently. Nothing this
            // app fetches is worth sending a driver's search terms in clear.
            throw new IllegalArgumentException("not https");
        }
        HttpURLConnection c = (HttpURLConnection) u.openConnection();
        c.setConnectTimeout(CONNECT_MS);
        c.setReadTimeout(READ_MS);
        c.setInstanceFollowRedirects(false);   // handled below, so the scheme can be checked
        c.setRequestProperty("User-Agent", userAgent);
        c.setRequestProperty("Accept-Encoding", "identity");
        return c;
    }

    // Read back through getters rather than exposed as fields: JNI reads a
    // static method far more simply than a static field, and mutable public
    // state on a class three threads can touch is worth not having.
    //
    // These describe THE LAST CALL, so they are only meaningful to a caller that
    // reads them before another call can overwrite them. That holds because only
    // one thread ever calls getText/getBytes — Carnyx's `carnyx-logos` worker —
    // and it reads all three in the same JNI attach as the fetch. The two
    // methods below (decode/encodePng) are the ones the UI thread calls, and
    // they deliberately touch none of this.
    private static int lastStatus;
    private static String lastContentType;
    private static String lastError;

    /** HTTP status of the last call, or 0 if it never got one. */
    public static int lastStatus() { return lastStatus; }
    /** Content-Type of the last call, or null. */
    public static String lastContentType() { return lastContentType; }
    /** Why the last call failed, or null if it did not. */
    public static String lastError() { return lastError; }

    private static byte[] get(String url, String userAgent, int maxBytes) {
        lastStatus = 0;
        lastContentType = null;
        lastError = null;
        String at = url;
        try {
            for (int hop = 0; hop <= MAX_REDIRECTS; hop++) {
                HttpURLConnection c = open(at, userAgent);
                try {
                    int code = c.getResponseCode();
                    lastStatus = code;
                    if (code == 301 || code == 302 || code == 303 || code == 307 || code == 308) {
                        String next = c.getHeaderField("Location");
                        if (next == null) { lastError = "redirect without a location"; return null; }
                        at = new URL(new URL(at), next).toString();
                        continue;
                    }
                    if (code < 200 || code > 299) return null;
                    lastContentType = c.getContentType();

                    // Read with the cap enforced as we go. Trusting Content-Length
                    // would let a lying server allocate whatever it liked.
                    InputStream in = c.getInputStream();
                    ByteArrayOutputStream out = new ByteArrayOutputStream();
                    byte[] buf = new byte[16 * 1024];
                    int n;
                    while ((n = in.read(buf)) > 0) {
                        out.write(buf, 0, n);
                        if (maxBytes > 0 && out.size() > maxBytes) {
                            lastError = "over the size cap";
                            return null;
                        }
                    }
                    return out.toByteArray();
                } finally {
                    c.disconnect();
                }
            }
            lastError = "too many redirects";
            return null;
        } catch (Throwable t) {
            String m = t.getMessage();
            lastError = (m == null || m.isEmpty()) ? t.getClass().getSimpleName() : m;
            return null;
        }
    }

    /** Fetch as UTF-8 text. Null on any failure; see lastStatus / lastError. */
    public static String getText(String url, String userAgent, int maxBytes) {
        byte[] b = get(url, userAgent, maxBytes);
        if (b == null) return null;
        return new String(b, java.nio.charset.StandardCharsets.UTF_8);
    }

    /** Fetch raw bytes. Null on any failure. */
    public static byte[] getBytes(String url, String userAgent, int maxBytes) {
        return get(url, userAgent, maxBytes);
    }

    /**
     * Decode to straight-alpha RGBA8, longest edge capped at {@code maxEdge}.
     *
     * Two passes: the bounds-only pass costs nothing and lets
     * {@code inSampleSize} halve a large image during decode rather than after,
     * which on this device is the difference between a brief pause and an
     * allocation that fails. The exact cap is then applied by Rust's resampler,
     * so the ladder maths stays in one place.
     *
     * Returns one byte array: a big-endian {@code int} width, a big-endian
     * {@code int} height, then straight RGBA. One JNI crossing instead of three,
     * and no shared-state handshake.
     *
     * BYTES, NOT INTS, and the difference is not cosmetic on this hardware. An
     * {@code int[]} of RGBA components costs four bytes per component — 16 MB for
     * a 1024×1024 logo here, and another 16 MB in the {@code Vec<i32>} Rust
     * copies it into. With the {@link Bitmap} and the intermediate ARGB array
     * still alive that is upwards of 40 MB in flight for one logo, on a 32-bit
     * head unit. As bytes the same image is 4 MB a side.
     */
    public static byte[] decode(byte[] bytes, int maxEdge) {
        if (bytes == null || bytes.length == 0) return null;
        try {
            BitmapFactory.Options probe = new BitmapFactory.Options();
            probe.inJustDecodeBounds = true;
            BitmapFactory.decodeByteArray(bytes, 0, bytes.length, probe);
            if (probe.outWidth <= 0 || probe.outHeight <= 0) return null;

            int sample = 1;
            int longest = Math.max(probe.outWidth, probe.outHeight);
            while (maxEdge > 0 && longest / (sample * 2) >= maxEdge) sample *= 2;

            BitmapFactory.Options opts = new BitmapFactory.Options();
            opts.inSampleSize = sample;
            // ARGB_8888 so alpha survives; a logo without transparency is rare.
            opts.inPreferredConfig = Bitmap.Config.ARGB_8888;
            Bitmap bm = BitmapFactory.decodeByteArray(bytes, 0, bytes.length, opts);
            if (bm == null) return null;
            try {
                int w = bm.getWidth(), h = bm.getHeight();
                if (w <= 0 || h <= 0) return null;
                int[] argb = new int[w * h];
                bm.getPixels(argb, 0, w, 0, 0, w, h);
                byte[] out = new byte[HEADER + w * h * 4];
                putInt(out, 0, w);
                putInt(out, 4, h);
                for (int i = 0; i < argb.length; i++) {
                    int p = argb[i];
                    int o = HEADER + i * 4;
                    out[o]     = (byte) (p >> 16);   // R
                    out[o + 1] = (byte) (p >> 8);    // G
                    out[o + 2] = (byte) p;           // B
                    out[o + 3] = (byte) (p >>> 24);  // A
                }
                return out;
            } finally {
                bm.recycle();
            }
        } catch (Throwable t) {
            return null;
        }
    }

    /** Two big-endian ints in front of the pixels: width, then height. */
    static final int HEADER = 8;

    private static void putInt(byte[] b, int at, int v) {
        b[at]     = (byte) (v >>> 24);
        b[at + 1] = (byte) (v >>> 16);
        b[at + 2] = (byte) (v >>> 8);
        b[at + 3] = (byte) v;
    }

    /** Encode straight-alpha RGBA8 as PNG. Null on failure. */
    public static byte[] encodePng(int w, int h, byte[] rgba) {
        if (w <= 0 || h <= 0 || rgba == null || rgba.length != w * h * 4) return null;
        try {
            int[] argb = new int[w * h];
            for (int i = 0; i < argb.length; i++) {
                int o = i * 4;
                argb[i] = ((rgba[o + 3] & 0xFF) << 24)
                        | ((rgba[o] & 0xFF) << 16)
                        | ((rgba[o + 1] & 0xFF) << 8)
                        | (rgba[o + 2] & 0xFF);
            }
            Bitmap bm = Bitmap.createBitmap(argb, w, h, Bitmap.Config.ARGB_8888);
            try {
                ByteArrayOutputStream out = new ByteArrayOutputStream();
                // PNG ignores the quality argument; it is lossless.
                if (!bm.compress(Bitmap.CompressFormat.PNG, 100, out)) return null;
                return out.toByteArray();
            } finally {
                bm.recycle();
            }
        } catch (Throwable t) {
            return null;
        }
    }
}
