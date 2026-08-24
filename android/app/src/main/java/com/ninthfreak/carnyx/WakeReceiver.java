package com.ninthfreak.carnyx;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.util.Log;

/**
 * Bring the face back when the head unit wakes.
 *
 * <h2>The event this is really for</h2>
 *
 * <p>{@code com.nwd.ACTION_OS_WAKE_UP} is the one that happens. THIS UNIT DOES
 * NOT COLD-BOOT ON AN IGNITION CYCLE — the MCU sleeps the SoC on ACC-off and
 * wakes it on ACC-on, so {@code BOOT_COMPLETED} fires roughly never on a
 * permanent install. It is declared anyway, as the fallback for a genuine cold
 * start, and the order in the manifest's filter is not the order of importance.
 *
 * <p>That vendor broadcasts reach an ordinary app at all is not an assumption:
 * {@code NwdBridge} already receives {@code com.nwd.action.ACTION_KEY_VALUE} for
 * the steering wheel, and the wheel works.
 *
 * <h2>Why the wake path is CONDITIONAL and the boot path is not</h2>
 *
 * <p>On a genuine boot there is nothing else the driver could have been doing,
 * so coming up is right. On a wake there is: if they were on maps or a music app
 * when the ignition went off, taking the foreground would be obnoxious. The flag
 * {@link CarnyxWake} keeps answers that, and it is the last thing the process
 * recorded before it was killed.
 *
 * <h2>Why this file is in android/ and not in java/</h2>
 *
 * <p>The same rule that puts {@link CarnyxService} here, and here it is not a
 * preference but the whole reason the receiver can exist. A manifest component
 * is constructed through the APPLICATION's class loader; the classes in
 * {@code java/} are dexed by {@code build.rs} and loaded at RUN time by an
 * {@code InMemoryDexClassLoader} that Rust builds after start-up. When this
 * broadcast arrives THE PROCESS IS DEAD and there is no Rust, no loader and no
 * dex — the platform starts a fresh process purely to deliver it. A class only
 * in the embedded dex would be a {@code ClassNotFoundException} at exactly the
 * moment it was needed.
 *
 * <p>The same fact is why the decision below is written in Java rather than
 * behind the Rust seam every other decision in this app lives behind: loading
 * {@code libcarnyx.so} to answer one boolean, in a process that exists for a few
 * milliseconds, would cost more than the feature.
 *
 * <p>Under cargo-apk this class is absent and cannot be added — that build
 * packages no Java and its manifest schema has no {@code <receiver>} field. The
 * app notices only by never seeing a wake note; see {@link CarnyxWake}.
 *
 * <h2>LOCKED_BOOT_COMPLETED is deliberately not here</h2>
 *
 * <p>CarFM's receiver handles it. It cannot arrive: that broadcast goes only to
 * components marked {@code android:directBootAware}, and nothing in this app is
 * — the face needs credential-encrypted storage for its own database before it
 * can draw anything. Declaring the filter without the flag would be a line that
 * looks like coverage and delivers nothing.
 *
 * <h2>The known risk</h2>
 *
 * <p>Android 10+ restricts starting an activity from the background, which is
 * exactly what this does. Head-unit ROMs are usually permissive and this one is
 * Android 10, but if the launch is refused there is no crash and no visible
 * effect. So the refusal is WRITTEN DOWN rather than swallowed — into the
 * shared preferences {@link CarnyxWake#takeLastWake} reads back, because a
 * {@code Log.w} on a unit with no adb reaches nobody.
 */
public final class WakeReceiver extends BroadcastReceiver {
    private static final String TAG = "CarnyxWake";

    /** The MCU's wake broadcast, from com.nwd.radio.service v214. */
    private static final String ACTION_OS_WAKE_UP = "com.nwd.ACTION_OS_WAKE_UP";
    private static final String ACTION_QUICKBOOT = "android.intent.action.QUICKBOOT_POWERON";

    /** Shared with {@link CarnyxWake} by name. Keep the two in step. */
    private static final String PREFS = "carnyx_wake";
    private static final String KEY_WAS_FOREGROUND = "was_foreground";
    private static final String KEY_LAST_WAKE = "last_wake";

    @Override
    public void onReceive(Context context, Intent intent) {
        if (context == null || intent == null || intent.getAction() == null) {
            return;
        }
        String action = intent.getAction();
        boolean conditional;
        if (Intent.ACTION_BOOT_COMPLETED.equals(action) || ACTION_QUICKBOOT.equals(action)) {
            conditional = false;
        } else if (ACTION_OS_WAKE_UP.equals(action)) {
            conditional = true;
        } else {
            // Something else matched the filter. Nothing to do, and nothing to
            // note either — a line per stray broadcast would drown the one that
            // matters.
            return;
        }

        if (conditional && !wasForeground(context)) {
            note(context, action + ": the face was not in front, left alone");
            return;
        }

        try {
            Intent launch = context.getPackageManager()
                    .getLaunchIntentForPackage(context.getPackageName());
            if (launch == null) {
                note(context, action + ": no launch intent for this package");
                return;
            }
            // NEW_TASK is not optional: a receiver has no task of its own to
            // start an activity into, and without the flag the platform throws.
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            context.startActivity(launch);
            note(context, action + ": brought the face forward");
        } catch (Throwable t) {
            // The likely one is a background-activity-start refusal. There is
            // nothing to recover — say which, so it is diagnosable rather than
            // indistinguishable from a broadcast that never came.
            note(context, action + ": launch refused: " + t);
        }
    }

    /** Was the face in front when the process was killed? See {@link CarnyxWake}. */
    private boolean wasForeground(Context context) {
        try {
            return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .getBoolean(KEY_WAS_FOREGROUND, false);
        } catch (Throwable t) {
            // Unreadable is not "yes". Coming forward over whatever the driver
            // is looking at, on a guess, is the one outcome worth avoiding.
            Log.w(TAG, "could not read the foreground flag: " + t);
            return false;
        }
    }

    /**
     * Leave one line for the app to find on its way up.
     *
     * <p>{@code commit()} rather than {@code apply()}, which is the opposite of
     * {@link CarnyxWake#setForeground}'s choice and for the opposite reason.
     * There, a blocking write sat on a lifecycle callback the platform was
     * waiting on. Here the process was started for this broadcast alone and may
     * be torn down the moment {@code onReceive} returns, so the write has to
     * have LANDED by then — an {@code apply()} whose background thread never got
     * scheduled would lose exactly the evidence this exists to produce.
     */
    private void note(Context context, String line) {
        Log.i(TAG, line);
        try {
            SharedPreferences p = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            p.edit().putString(KEY_LAST_WAKE, line).commit();
        } catch (Throwable t) {
            Log.w(TAG, "could not record the wake note: " + t);
        }
    }
}
