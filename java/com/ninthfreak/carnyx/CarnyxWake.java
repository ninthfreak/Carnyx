package com.ninthfreak.carnyx;

import android.content.Context;
import android.content.SharedPreferences;
import android.provider.Settings;
import android.util.Log;

/**
 * Remembers whether the face was in front, for the receiver that has to decide
 * whether to bring it back.
 *
 * <h2>Why a flag at all</h2>
 *
 * <p>The MCU sleeps the SoC on ACC-off and kills this process while it is down.
 * On ACC-on {@code WakeReceiver} has to answer one question — should Carnyx come
 * forward? — and the process that knew is gone. So the answer is written down
 * BEFORE it dies, and the receiver reads it back.
 *
 * <p>On a genuine boot there is nothing else the driver could have been doing, so
 * that path is unconditional and never consults this. On a WAKE there is: if
 * they were on maps or a music app when the ignition went off, taking the
 * foreground would be obnoxious.
 *
 * <h2>Two trees, one file, no shared class</h2>
 *
 * <p>This writer is in {@code java/} — the runtime dex, where Rust can reach it.
 * {@code WakeReceiver} is in the GRADLE source set, because a manifest component
 * must be constructible by the application's class loader. They never reference
 * each other: both name the same SharedPreferences file and key, which is a
 * platform store either side can open on its own. The strings are duplicated
 * deliberately, the way {@code CarnyxProcess} names its service by string.
 *
 * <h2>Written on RESUME and PAUSE, and that is the whole trick</h2>
 *
 * <p>Nothing writes it "on the way down", because there may be no way down —
 * the kill can be abrupt and deliver no lifecycle callback at all. Instead the
 * flag is kept CURRENT: true whenever the face is in front, false the moment it
 * is not. Whatever it holds when the process dies is the honest answer to what
 * the driver was looking at.
 *
 * <h2>The traffic runs both ways</h2>
 *
 * <p>{@link #takeLastWake} reads back the note {@code WakeReceiver} leaves when
 * it fires. THAT IS THE ONLY EVIDENCE THIS FEATURE CAN EVER PRODUCE. Everything
 * the receiver does happens in a process with no face, on a unit with no adb,
 * so a {@code Log.i} from it reaches nobody; the app reads the note on its way
 * up and puts it in the diagnostics log, which is a channel a driver can
 * actually see. Without it, "the broadcast never arrived" and "the launch was
 * refused" look identical — the app is simply not there.
 */
public final class CarnyxWake {
    private static final String TAG = "CarnyxWake";

    /** Shared with {@code WakeReceiver} by name. Keep the two in step. */
    private static final String PREFS = "carnyx_wake";
    private static final String KEY_WAS_FOREGROUND = "was_foreground";
    private static final String KEY_LAST_WAKE = "last_wake";

    /**
     * Shared with {@code SleepReceiver} by name, and holding the other
     * direction's traffic.
     *
     * <p>{@code last_sleep} is what the sleep left behind; {@code
     * release_on_sleep} is the driver's switch, written here so a receiver in a
     * COLD PROCESS can honour it. That receiver has no app, no settings file
     * read and no Rust — this is the only way it can know the driver turned the
     * feature off.
     */
    private static final String KEY_LAST_SLEEP = "last_sleep";

    /**
     * What the notification listener's last bind or unbind did.
     *
     * <p>Written by {@code CarnyxListener}, which shares this file BY NAME the
     * way {@code SleepReceiver} does — the platform can bind that service into a
     * process with no Rust in it, so the two halves can never share a constant.
     */
    private static final String KEY_LAST_LISTENER = "last_listener";

    /**
     * The driver's "come forward when the platform binds us" switch.
     *
     * <p>Default FALSE and read by a cold process, for {@code
     * KEY_RELEASE_ON_SLEEP}'s reason: the listener may be bound with no app
     * behind it and no settings file parsed, and this is the only way it can
     * know what the driver asked for.
     */
    private static final String KEY_COME_FORWARD = "come_forward";
    private static final String KEY_RELEASE_ON_SLEEP = "release_on_sleep";

    private static Context ctx;

    private CarnyxWake() {
    }

    /** Hand the class the app context, as {@link CarnyxProcess#attach} does. */
    public static synchronized void attach(Context context) {
        if (ctx == null && context != null) {
            ctx = context.getApplicationContext();
        }
    }

    /**
     * Record whether the face is in front.
     *
     * <p>{@code apply()} rather than {@code commit()}: this runs from a lifecycle
     * callback that the platform is BLOCKING on, and a synchronous disk write
     * there is the shape that turns a pause into a dropped frame. {@code apply}
     * writes to memory immediately and to disk on a background thread; the flag
     * is only ever read after a process death, by which time the write has long
     * landed.
     *
     * <p>NOT the Java main thread, which an earlier version of this note claimed.
     * The call arrives on the NATIVE thread running {@code android_main} — Slint
     * delivers it through {@code init_with_event_listener} — while the Java main
     * thread waits for that thread to acknowledge the command. The reasoning is
     * unchanged, because what matters is that something is blocked on this
     * returning, not which thread is blocked.
     */
    public static synchronized void setForeground(boolean front) {
        if (ctx == null) {
            return;
        }
        try {
            SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            p.edit().putBoolean(KEY_WAS_FOREGROUND, front).apply();
        } catch (Throwable t) {
            Log.w(TAG, "could not record the foreground flag: " + t);
        }
    }

    /**
     * What {@code WakeReceiver} did last time, and forget it.
     *
     * <p>Returns {@code ""} when the receiver has said nothing since the app
     * last asked — which is the ordinary answer on a launcher tap, and the
     * ordinary answer on a cargo-apk build where there is no receiver at all.
     *
     * <p>TAKEN, NOT READ, and the clear is the point. This note is a record of
     * ONE start-up; leaving it in place would put the same line in the
     * diagnostics log of every launch that followed, and a stale
     * "brought forward by the wake broadcast" on a launch the driver did by hand
     * is worse than no line, because it is the sort of thing a later session
     * reasons from.
     */
    public static synchronized String takeLastWake() {
        return take(KEY_LAST_WAKE, "wake");
    }

    /**
     * What the last SLEEP managed, and forget it.
     *
     * <p>THE DIAGNOSTICS LOG CANNOT CARRY THIS AND NEVER COULD. It is a ring in
     * memory — {@code prefs.rs} says so in its opening note, and
     * {@code crashlog.rs} was built on the same fact — so a line written as the
     * MCU cuts power dies with the process that wrote it. The owner's report
     * that Carnyx "doesn't turn the radio off at sleep" was therefore
     * unanswerable from the log: the run that would have recorded it was gone by
     * the time anyone could read it. This is that line, on disk, taken on the
     * way back up.
     *
     * <p>An EMPTY answer is itself the finding, and the reason the app prints a
     * line either way. Nothing recorded, on a launch that the wake note says
     * followed an ignition cycle, means the ACC-off broadcast never arrived —
     * which is a different fault from a release that was attempted and failed,
     * and needs a different fix.
     */
    public static synchronized String takeLastSleep() {
        return take(KEY_LAST_SLEEP, "sleep");
    }

    /**
     * What the notification listener last did, and forget it.
     *
     * <p>THE ONE LINE THAT SAYS WHETHER OUTCOME C IS REACHABLE. A listener note
     * present after an ignition cycle means the PLATFORM started this app's
     * process with no human involved — which every broadcast receiver here has
     * failed to achieve, because the vendor force-stops the package first.
     * Absent, and the force-stop takes the listener down with everything else.
     *
     * <p>Empty is also what a cargo-apk build always reports: that packager
     * declares no services, so there is no listener to bind.
     */
    public static synchronized String takeLastListener() {
        return take(KEY_LAST_LISTENER, "listener");
    }

    /**
     * Whether the driver has granted notification access.
     *
     * <p>Read off {@code Settings.Secure}'s own list rather than inferred from
     * whether {@code CarnyxListener} has ever been constructed: the app needs
     * this while drawing a settings row, which is a moment when the platform has
     * told it nothing. The keep-alive probe reads the same key for its report;
     * this is the same question asked where a row can act on the answer.
     *
     * <p>SUBSTRING, NOT EQUALITY. The setting is a colon-separated list of
     * flattened component names across every app that holds the grant.
     */
    public static synchronized boolean isListenerGranted() {
        if (ctx == null) {
            return false;
        }
        try {
            String enabled = Settings.Secure.getString(
                    ctx.getContentResolver(), "enabled_notification_listeners");
            return enabled != null && enabled.contains(ctx.getPackageName());
        } catch (Throwable t) {
            Log.w(TAG, "could not read the listener grant: " + t);
            return false;
        }
    }

    /** Read one note and clear it. See {@link #takeLastWake}. */
    private static String take(String key, String what) {
        if (ctx == null) {
            return "";
        }
        try {
            SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            String note = p.getString(key, "");
            if (note != null && !note.isEmpty()) {
                p.edit().remove(key).apply();
            }
            return note == null ? "" : note;
        } catch (Throwable t) {
            Log.w(TAG, "could not read the " + what + " note: " + t);
            return "";
        }
    }

    /**
     * Leave one line about what the sleep managed, from whichever receiver heard
     * it.
     *
     * <p>{@code commit()} rather than {@code apply()}, for {@code
     * WakeReceiver.note}'s reason turned around: the MCU has just announced it
     * is cutting power to the SoC and this app holds no wake lock, so an
     * {@code apply()} whose background thread never got scheduled would lose
     * exactly the evidence this exists to produce. A blocking write here is
     * measured against a process that may not exist a moment from now.
     */
    public static synchronized void noteSleep(String line) {
        if (ctx == null || line == null || line.isEmpty()) {
            return;
        }
        try {
            ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .edit().putString(KEY_LAST_SLEEP, line).commit();
        } catch (Throwable t) {
            Log.w(TAG, "could not record the sleep note: " + t);
        }
    }

    /**
     * Write the driver's release-on-sleep switch where a cold process can read
     * it.
     *
     * <p>{@code SleepReceiver} runs in a process with no app behind it: no
     * prefs.json read, no Rust, no settings. Without this it would release the
     * source on every ACC-off regardless of what the driver asked for. Written
     * on every move of the switch and at start-up, so the stored value is never
     * a guess about a run that has since changed its mind.
     *
     * <p>{@code commit()} for the same reason as {@link #noteSleep}: the switch
     * is read by a process that starts while this one is being killed.
     */
    public static synchronized void setReleaseOnSleep(boolean on) {
        if (ctx == null) {
            return;
        }
        try {
            ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .edit().putBoolean(KEY_RELEASE_ON_SLEEP, on).commit();
        } catch (Throwable t) {
            Log.w(TAG, "could not record the release switch: " + t);
        }
    }

    /**
     * Write the driver's come-forward switch where the notification listener can
     * read it.
     *
     * <p>Exactly {@link #setReleaseOnSleep}'s arrangement and for exactly its
     * reason: {@code CarnyxListener} may be bound by the platform into a process
     * with no app behind it, so a switch it must honour cannot live in
     * {@code prefs.json}.
     */
    public static synchronized void setComeForward(boolean on) {
        if (ctx == null) {
            return;
        }
        try {
            ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .edit().putBoolean(KEY_COME_FORWARD, on).commit();
        } catch (Throwable t) {
            Log.w(TAG, "could not record the come-forward switch: " + t);
        }
    }
}
