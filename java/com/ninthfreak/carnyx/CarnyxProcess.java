package com.ninthfreak.carnyx;

import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.os.SystemClock;
import android.text.format.DateFormat;
import android.util.Log;

/**
 * Starts and stops {@link CarnyxService} — from the RUNTIME dex, across the gap
 * between this app's two Java trees.
 *
 * <p>This file is in {@code java/}: compiled by {@code build.rs}, dexed, embedded
 * in the library and loaded by an {@code InMemoryDexClassLoader}. The service it
 * starts is in {@code android/app/src/main/java/}, compiled by AGP into the APK's
 * own dex, because a manifest-declared component has to be constructible by the
 * application's class loader. Two trees, one process.
 *
 * <p>(Under the Gradle build this file is compiled TWICE — AGP has
 * {@code ../../java} in its java source set too, so it is in the APK dex as well
 * as the embedded one, and Android's parent-first loading means the APK copy is
 * the one that runs. Same source either way. Under cargo-apk only the embedded
 * copy exists.)
 *
 * <p>WHICH IS WHY THE INTENT NAMES THE SERVICE AS A STRING. Writing
 * {@code new Intent(ctx, CarnyxService.class)} would make this file depend at
 * COMPILE time on a class that is not on its compile path and at RUN time on the
 * runtime loader being able to resolve into the APK's dex. {@code ComponentName}
 * with a class-name string asks the package manager instead, which is the one
 * component that can see both — and it turns "the class is not in this build"
 * into a caught exception rather than a link error.
 *
 * <p>THAT CASE IS REAL AND EXPECTED. cargo-apk packages no Java at all, so under
 * the default build {@link CarnyxService} genuinely does not exist and the start
 * below fails every time. That is not a fault to fix here: it is why #67 needed a
 * different packager. Build with {@code tools/build-apk-gradle.sh} and the class
 * is there. Either way the app runs; only the pinning is lost.
 */
public final class CarnyxProcess {
    private static final String TAG = "CarnyxProcess";

    /** Must match the class AGP compiles into the APK. */
    private static final String SERVICE = "com.ninthfreak.carnyx.CarnyxService";

    /**
     * The APPLICATION context, as {@link NwdBridge} holds one and for the same
     * reason: holding it statically leaks nothing, and Rust is then never in the
     * business of keeping a Java object reference alive across calls.
     */
    private static Context ctx;

    private CarnyxProcess() {
    }

    /**
     * Hand the class the app context. Called once from Rust at start-up with the
     * NativeActivity; {@code getApplicationContext()} is taken so nothing here
     * can outlive and leak an Activity.
     */
    public static synchronized void attach(Context context) {
        if (ctx == null && context != null) {
            ctx = context.getApplicationContext();
        }
    }

    /**
     * Ask the platform to run the service in the foreground.
     *
     * <p>Called from Rust at start-up, while the activity is on screen — which
     * matters: from Android 12 a background {@code startForegroundService} throws
     * {@code ForegroundServiceStartNotAllowedException}, and start-up is exactly
     * the moment when the app is indisputably in the foreground.
     *
     * @return true when the platform accepted the start. It is not a promise
     *     that the service entered the foreground — that happens later, on the
     *     main thread, and {@code CarnyxService} logs its own failure.
     */
    public static synchronized boolean start(String text) {
        if (ctx == null) {
            Log.i(TAG, "start() before attach()");
            return false;
        }
        Intent intent = new Intent();
        intent.setComponent(new ComponentName(ctx.getPackageName(), SERVICE));
        if (text != null) {
            intent.putExtra("com.ninthfreak.carnyx.extra.TEXT", text);
        }
        try {
            if (Build.VERSION.SDK_INT >= 26) {
                ctx.startForegroundService(intent);
            } else {
                ctx.startService(intent);
            }
            Log.i(TAG, "foreground service requested");
            return true;
        } catch (Exception e) {
            // The expected failure is the cargo-apk build, where the class is
            // absent and the package manager refuses the component. Logged at
            // info, not error: on that build it is the designed outcome.
            Log.i(TAG, "no foreground service on this build: " + e);
            return false;
        }
    }

    /** Stop it. Nothing calls this yet; the service dies with the process. */
    public static synchronized void stop() {
        if (ctx == null) {
            return;
        }
        Intent intent = new Intent();
        intent.setComponent(new ComponentName(ctx.getPackageName(), SERVICE));
        try {
            ctx.stopService(intent);
        } catch (Exception e) {
            Log.i(TAG, "stopService: " + e);
        }
    }

    // ── THE CLOCK'S TWO FACTS (§4.8) ─────────────────────────────────────────
    //
    // Here rather than in a class of their own because this one already holds
    // the application context and a clock is two reads off it. Both return
    // FACTS, not strings: what to do with a 20 and a 5 — whether it is `08:05`,
    // `!8:05 A`, or nothing at all — is `crate::clock`, where it is tested.

    /**
     * How long this PROCESS had already been running, in seconds, or -1.
     *
     * <h2>What this settles, and why nothing else could</h2>
     *
     * <p>The question #133 turns on is whether the platform starts Carnyx AFTER
     * the vendor force-stops it on ACC-off. A note saying "bound by the platform"
     * cannot answer it: tapping a force-stopped app's icon makes Android reinstate
     * that package's components at that moment, so the bind can happen a fraction
     * of a second before the note is read, as part of the launch itself.
     *
     * <p>This tells the two apart with one number. If the driver taps the icon and
     * the process turns out to have been alive for forty seconds already, then
     * SOMETHING ELSE STARTED IT — the notification listener, or a manifest
     * receiver — and the launch merely attached a window to a process that was
     * running. A fraction of a second means the tap started it and nothing had.
     *
     * <p>ELAPSED REALTIME ON BOTH SIDES, which is the clock that keeps counting
     * while the unit is suspended. The wall clock would be wrong across a time
     * sync and {@code uptimeMillis} stops during sleep, and this unit's whole
     * problem happens across a sleep.
     *
     * <p>-1 rather than 0 when it cannot be read: zero is a real answer here — it
     * is the answer for every ordinary tap — so a failure must not look like one.
     */
    public static long processAgeSeconds() {
        try {
            long ms = SystemClock.elapsedRealtime() - android.os.Process.getStartElapsedRealtime();
            return ms < 0 ? -1 : ms / 1000L;
        } catch (Throwable t) {
            Log.w(TAG, "could not read the process start time", t);
            return -1;
        }
    }

    /**
     * The local wall clock as {@code hour * 100 + minute}, or -1.
     *
     * <p>ONE INT RATHER THAN TWO CALLS, because the two must come from the SAME
     * reading: asked separately, a call that straddles 09:59→10:00 returns hour
     * 9 and minute 0, and the face shows 09:00 for a minute. `Calendar` is read
     * once and both fields taken off it.
     *
     * <p>{@code Calendar.getInstance()} and not {@code LocalTime}: this dex is
     * built for API 26 and {@code java.time} is API 26+ ONLY WITH desugaring,
     * which {@code build.rs}'s d8 invocation does not turn on.
     */
    public static int clockHourMinute() {
        try {
            java.util.Calendar c = java.util.Calendar.getInstance();
            return c.get(java.util.Calendar.HOUR_OF_DAY) * 100 + c.get(java.util.Calendar.MINUTE);
        } catch (Throwable t) {
            Log.w(TAG, "could not read the clock", t);
            return -1;
        }
    }

    /**
     * Is the system set to 24-hour time?
     *
     * <p>ASKED EVERY TICK AND NEVER STORED. §4.8: the readout "re-formats on
     * every tick, so flipping the system toggle in Settings ▸ System ▸ Date &
     * time changes the radio face with no restart and no app-side preference to
     * keep in sync". An app-side copy of this would be a second source of truth
     * for a fact Android already owns.
     *
     * <p>Defaults to FALSE with no context, which is the same answer a US-locale
     * device gives — a wrong guess here shows a meridiem that should not be
     * there rather than hiding one that should.
     */
    public static boolean clockIs24Hour() {
        try {
            return ctx != null && DateFormat.is24HourFormat(ctx);
        } catch (Throwable t) {
            Log.w(TAG, "could not read the 12/24 setting", t);
            return false;
        }
    }

    /**
     * The device's ISO 3166-1 alpha-2 country, or {@code ""}.
     *
     * <p>ONE FACT, AND NO DECISION. Whether that country signs its roads in
     * miles is {@code crate::units::Units::for_country}'s, for the rule this
     * tree states everywhere: a table written here could not be tested on a
     * machine with no head unit, and the one there is tested against the cases
     * that actually catch it (Britain signs in miles, Ireland does not).
     *
     * <p>THE CONFIGURATION'S LOCALE AND NOT {@code Locale.getDefault()}, which
     * is the process-wide default and does not follow a locale change until
     * something re-reads it. The resources' configuration is what the rest of
     * the system is using right now, so a driver who switches the phone to a US
     * locale gets feet at the next poll rather than at the next cold start.
     *
     * <p>Empty is a real answer: a locale can carry a language and no region
     * ({@code "en"}), and {@code getCountry()} returns {@code ""} for it. Metric
     * is what that becomes, which is the safer of the two guesses.
     */
    public static String countryCode() {
        try {
            if (ctx == null) {
                return "";
            }
            java.util.Locale l;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                l = ctx.getResources().getConfiguration().getLocales().get(0);
            } else {
                l = ctx.getResources().getConfiguration().locale;
            }
            return l == null ? "" : l.getCountry();
        } catch (Throwable t) {
            Log.w(TAG, "could not read the locale", t);
            return "";
        }
    }

    /**
     * What kind of build this ROM is, and whether {@code adb root} could work.
     *
     * <h2>WHY THE APP REPORTS THIS AT ALL</h2>
     *
     * <p>The kernel source remap (#133) has to be written into {@code /config},
     * and 2026-09-25 established that nothing reachable from ordinary app code
     * can do it — the vendor's own file copier runs as an ordinary uid and its
     * {@code mkdirs} on {@code /config/app} failed. What is left needs a
     * privileged shell, and on Android that means {@code adb root}, which only
     * works on a {@code userdebug} or {@code eng} build (or a {@code user} build
     * left with {@code ro.debuggable=1}).
     *
     * <p>THIS SAVES A TRIP TO THE CAR. Getting adb onto a head unit in a driveway
     * is awkward — the right cable, the right port, developer options — and doing
     * all of that only to be told {@code adbd cannot run as root in production
     * builds} is the wasted evening this line exists to prevent. These are
     * ordinary system properties any app may read.
     *
     * <p>{@code Build.TYPE} and {@code Build.TAGS} are public API.
     * {@code ro.debuggable} is not, so it goes through reflection on
     * {@code SystemProperties} — the same route `NwdBridge` uses for the vendor
     * framework classes — and its absence is reported rather than guessed at.
     *
     * @return one line for the diagnostics log, never null.
     */
    public static String buildKind() {
        String type;
        String tags;
        try {
            type = Build.TYPE == null ? "?" : Build.TYPE;
            tags = Build.TAGS == null ? "?" : Build.TAGS;
        } catch (Throwable t) {
            return "build: could not be read (" + t + ")";
        }
        String debuggable;
        try {
            Class<?> sp = Class.forName("android.os.SystemProperties");
            Object v = sp.getMethod("get", String.class, String.class)
                    .invoke(null, "ro.debuggable", "");
            debuggable = v == null ? "" : v.toString();
        } catch (Throwable t) {
            debuggable = "";
        }
        // THE VERDICT IS SPELLED OUT rather than left to whoever reads the log.
        // "userdebug" means nothing to most people and the whole point of the
        // line is to answer one question: is it worth carrying a cable out there.
        boolean rootable = "userdebug".equals(type) || "eng".equals(type) || "1".equals(debuggable);
        return "build: " + type + "/" + tags
                + (debuggable.isEmpty() ? "" : ", ro.debuggable=" + debuggable)
                + (rootable
                        ? " — adb root should work"
                        : " — adb root will be refused on a build of this kind");
    }
}
