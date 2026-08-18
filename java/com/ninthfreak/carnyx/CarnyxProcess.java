package com.ninthfreak.carnyx;

import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
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
}
