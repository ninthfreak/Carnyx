package com.ninthfreak.carnyx;

import android.content.Context;
import android.content.SharedPreferences;
import android.os.SystemClock;
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
 * the receiver does happens in a process with no face, where logcat reaches nobody,
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

    /**
     * Whether FM was the MCU's audio source when this app last shut down.
     *
     * <h2>THE CONDITION THE OWNER RANKED FIRST, AND THE ONE THE FIRST BUILD
     * IGNORED</h2>
     *
     * <p>#133's outcome A is "how it works currently, except that it would launch
     * Carnyx instead of the stock app" — and "how it works currently" INCLUDES
     * the condition: *"If the radio wasn't playing when the unit went to sleep,
     * the stock radio app doesn't get launched."* The first come-forward build
     * came forward on every platform bind, which is not A, not B and not C; it
     * put the face on screen after an ignition cycle in which nothing had been
     * playing. The owner: *"This is terrible behavior for the head unit."*
     *
     * <p>MEASURED, NOT GUESSED. Written at shutdown from {@code mcuSource()} —
     * the MCU's own current-source number, 4 being FM — so it records what the
     * hardware was actually doing rather than what this app believed. Written
     * AFTER any release, so it also answers the question that matters next:
     * releasing means the radio is off and nothing should come forward, and
     * leaving it alone means the vendor will resume FM and the stock app with it.
     *
     * <p>ABSENT MEANS FALSE, and that is a deliberate default rather than an
     * accident of the API. A missing key is a unit where this app has never shut
     * down cleanly, and the safe answer there is silence: a face that fails to
     * appear is a disappointment, and a face that appears over a driver's map is
     * the defect being fixed.
     */
    private static final String KEY_RADIO_PLAYING = "radio_playing";

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
        // ── COMING TO THE FRONT RETIRES ANY SIGHTING ────────────────────────
        //
        // THE PATH THIS CLOSES: `SLEEP_ACTIONS` includes `ACTION_SCREEN_OFF`, so
        // a screen blank with FM playing takes a sighting and writes the durable
        // flag. If the screen then comes back, the driver switches FM off, and
        // the unit kills the app later without a Destroy, nothing ever overwrote
        // that flag — and the listener brings the face forward citing a radio
        // that was turned off half an hour earlier. The reasserting-window
        // defect again, by its last remaining road.
        //
        // A driver looking at the face is the proof the sighting is spent: the
        // app is alive and in front, so whatever happens next will be recorded
        // by the shutdown that follows it. The 30-second window covers the
        // in-memory half; this covers the durable one.
        if (front) {
            fmAtSleepElapsed = Long.MIN_VALUE;
            setRadioPlaying(false);
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
        append(KEY_LAST_SLEEP, line);
    }

    /**
     * How many entries one key keeps. See {@code CarnyxNotes.KEEP}, which states
     * the same number on the other side of the class-loader divide.
     */
    private static final int KEEP = 8;

    /** Entries are joined by this. Nothing written here contains one. */
    private static final String SEP = "\n";

    /**
     * Append one line to a key's ring.
     *
     * <p>A RING, NOT A SLOT. Each note used to be one value, overwritten, and
     * {@link #take} clears on read — so a note appeared in the log of the FIRST
     * launch after the event and was erased at that moment. If that session died
     * before the driver exported, the evidence was gone: the diagnostics log is
     * a ring in memory and does not survive the process either. On a unit where
     * the experiment IS "switch the car off, switch it on, see what was
     * recorded", that put the answer one mis-step from being lost.
     *
     * <p>THE RULE IS STATED TWICE, here and in {@code CarnyxNotes}, and it has
     * to be: that class is in the Gradle source set and this one is dexed by
     * {@code build.rs} and loaded by an {@code InMemoryDexClassLoader}. The two
     * halves can never meet in memory — the same divide {@link #PREFS} is
     * already shared by name across.
     *
     * <p>{@code commit()} for {@link #noteSleep}'s original reason: the MCU has
     * announced it is cutting power and this app holds no wake lock.
     */
    private static void append(String key, String line) {
        if (ctx == null || line == null || line.isEmpty()) {
            return;
        }
        line = stamp() + "  " + line;
        try {
            SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            String prev = p.getString(key, "");
            String joined = prev == null || prev.isEmpty() ? line : prev + SEP + line;
            String[] parts = joined.split(SEP, -1);
            if (parts.length > KEEP) {
                StringBuilder b = new StringBuilder();
                for (int i = parts.length - KEEP; i < parts.length; i++) {
                    if (b.length() > 0) {
                        b.append(SEP);
                    }
                    b.append(parts[i]);
                }
                joined = b.toString();
            }
            p.edit().putString(key, joined).commit();
        } catch (Throwable t) {
            Log.w(TAG, "could not record a " + key + " note: " + t);
        }
    }

    /**
     * The wall clock, and how long this unit has been asleep since it booted.
     *
     * <p>THE THIRD THING STATED TWICE ACROSS THE DIVIDE, after {@link #PREFS} and
     * the appending rule above, and for the same unavoidable reason: this class is
     * dexed by {@code build.rs} and loaded by an {@code InMemoryDexClassLoader},
     * while {@code CarnyxNotes} is in the Gradle source set. The two can never
     * meet in memory. {@code CarnyxNotes.stamp} carries the full argument for what
     * this measures and why an undated note answered nothing; this is that method,
     * on this side.
     *
     * <p>The FORMAT is shared by name as much as the file is: a reader looking at
     * `wake:` and `listener:` lines in one log must not have to learn two.
     */
    private static String stamp() {
        StringBuilder b = new StringBuilder();
        try {
            java.util.Calendar c = java.util.Calendar.getInstance();
            b.append(String.format(java.util.Locale.US, "%02d:%02d:%02d",
                    c.get(java.util.Calendar.HOUR_OF_DAY),
                    c.get(java.util.Calendar.MINUTE),
                    c.get(java.util.Calendar.SECOND)));
        } catch (Throwable t) {
            b.append("--:--:--");
        }
        try {
            long ms = SystemClock.elapsedRealtime() - SystemClock.uptimeMillis();
            long m = ms / 60000L;
            b.append(" slept ").append(m < 60 ? m + "m" : (m / 60) + "h" + (m % 60) + "m");
        } catch (Throwable t) {
            b.append(" slept ?");
        }
        return b.toString();
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
        // ── TURNING IT ON DISCARDS ANY SIGHTING FROM WHILE IT WAS OFF ────────
        //
        // `radio_playing` is spent on read by `CarnyxListener`, but ONLY on the
        // path where come-forward is on — the switch is tested first and returns
        // before the consume. So a sighting taken while the switch was off is
        // never spent and simply waits. Flip the switch on a week later and the
        // very next platform bind acts on it: the face arrives over whatever the
        // driver is doing, citing a radio that stopped playing days ago. That is
        // the reasserting-window defect with a longer fuse.
        //
        // ONLY ON THE WAY ON. Turning the switch OFF leaves the flag alone,
        // because nothing reads it while it is off and the next shutdown
        // overwrites it anyway.
        if (on) {
            fmAtSleepElapsed = Long.MIN_VALUE;
            setRadioPlaying(false);
        }
    }

    /**
     * Hand the FM source back, then record what the MCU is left on.
     *
     * <p>CALLED FROM `Destroy`, WHICH IS THE CALLBACK THIS UNIT ACTUALLY GIVES.
     * The vendor ACC-off broadcast this app spent three builds waiting for has
     * never arrived in any log; the ordinary Android teardown has. The
     * 2026-09-10 log read `last run ended in destroy 46326s ago`, landing at
     * 18:29:10, against a screenshot putting the replacement process at 18:29:12.
     * The unit tears the app down and then takes the package.
     *
     * <p>DESTROY AND NOT PAUSE OR STOP. Those two are what BACKGROUNDING looks
     * like — the driver in maps, the screen on a timer — and the radio is meant
     * to play through both. This is the app going away. The owner asked for
     * exactly that boundary and named the precedent: *"The stock app will kill
     * the radio when closed by Android, whether or not Carnyx is running."*
     * Closed BY ANDROID counts, which is why nothing here tries to tell a
     * user-initiated close from a system one.
     *
     * <p>See {@link #KEY_RADIO_PLAYING} for what the recorded half is for.
     *
     * <p>{@code commit()} and not {@code apply()}, for {@code CarnyxNotes}'
     * reason: the MCU is cutting power and an {@code apply()} whose background
     * thread never got scheduled would lose the one fact the next launch needs.
     *
     * @return the line for the diagnostics log. Never null.
     */
    public static synchronized String onAppDestroyed() {
        if (ctx == null) {
            return "shutdown: no context";
        }

        // ── READ THE MCU FIRST, RELEASE SECOND, AND NEVER READ IT AGAIN ──────
        //
        // THE FIRST CUT OF THIS METHOD READ IT AFTERWARDS and was wrong on the
        // one path that matters. `releaseSource` ends in `ctx.sendBroadcast` —
        // fire-and-forget. The vendor service in another process has still to be
        // dispatched, command the MCU, and have the MCU write
        // `mcu_current_source` back. A re-read microseconds later therefore
        // still returns 4, so the release-on path recorded `radio_playing=true`:
        // the face came forward after an ignition cycle in which the radio had
        // just been deliberately handed back. Exactly the behaviour the gate
        // exists to prevent. This file measures that lag itself, four hundred
        // lines up — *"the MCU re-powered FM a second later"*.
        //
        // So the state is read ONCE, before anything is sent, and what gets
        // recorded is derived rather than re-measured: FM will be playing into
        // the sleep if it was playing AND we did not hand it back.
        int src;
        try {
            src = NwdBridge.mcuSource();
        } catch (Throwable t) {
            // UNKNOWN IS RECORDED AS NOT PLAYING. See KEY_RADIO_PLAYING: the
            // wrong answer in this direction costs a face that did not appear,
            // and in the other it costs the defect being fixed.
            setRadioPlaying(false);
            return note("shutdown: could not read the MCU source, recorded as off ("
                    + t + ")");
        }
        boolean wasPlaying = src == 4;

        // FALSE ON A FAILED OR ABSENT READ, matching the shipped default since
        // #133. An unreadable key means the app has never pushed the mirror,
        // which on a fresh install means it has barely run — and the default it
        // would have pushed is off, because the kernel remap is the intended way
        // to stop the relaunch and the handback fights it. See
        // `Settings::release_on_sleep`.
        boolean on = false;
        try {
            on = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .getBoolean(KEY_RELEASE_ON_SLEEP, false);
        } catch (Throwable t) {
            Log.w(TAG, "could not read the release switch: " + t);
        }

        String released;
        boolean handedBack = false;
        if (!on) {
            released = " — release is off";
        } else if (!wasPlaying) {
            // `releaseSource` would reach its own ownership test and send
            // nothing, so this says the same thing without the round trip — and
            // without a "skipped" line that reads like a failure.
            released = " — nothing to release";
        } else {
            // ── BOTH ROUTES, AND THIS IS THE PATH THAT MOST NEEDS THE FAST ONE ──
            //
            // THIS HOOK IS THE ONE WITH EVIDENCE OF RUNNING AT ACC-OFF. The
            // drive log carries `destroy 39s ago` recorded across an ignition
            // cycle: the unit tears the window down and THEN force-kills, so
            // Destroy fires on the way into the sleep. The two sleep receivers
            // in `NwdBridge` are the paths whose ACC-off delivery is still a
            // hypothesis — nobody has yet seen either fire on this ROM.
            //
            // For three commits this path called `releaseSource` alone, which is
            // the broadcast: queued by ActivityManager and delivered to a third
            // process, measured to work on a manual close and NOT at ACC-off.
            // The synchronous kernel call went to the two receivers that might
            // never run, and the one that does run kept the route that loses the
            // race. `handBackNow` sends both, kernel first.
            try {
                released = " — " + NwdBridge.handBackNow();
                handedBack = true;
            } catch (Throwable t) {
                released = " — release failed: " + t;
            }
        }

        // THE RACE HERE FAILS SAFE, which is why a non-throwing call is taken as
        // a handover. If the source moved between the read above and
        // `releaseSource`'s own ownership test, this records "not playing" for a
        // radio that is — and the cost of that is a face that does not come
        // forward. The opposite mistake is the defect being fixed.
        //
        // AND A SLEEP ROUTE MAY HAVE GOT HERE FIRST, which is the 2026-09-17
        // defect. By the time this hook runs, the state observer and the runtime
        // watch have usually already handed the source back, so `wasPlaying` is
        // read off an MCU this app has itself just changed. `noteFmAtSleep` is
        // the sighting they took BEFORE doing it. See that method.
        boolean playing = (wasPlaying && !handedBack) || fmSeenAtSleepRecently();
        setRadioPlaying(playing);
        // "WAS PLAYING" AND NOT "LEFT PLAYING", which is the wording this line
        // carried while it meant the other thing. The flag now answers what the
        // driver had on going into the sleep, not what this app left behind.
        return note("shutdown: " + (playing ? "FM was playing" : "FM not playing")
                + " (mcu_current_source was " + src
                + (fmSeenAtSleepRecently() ? ", FM seen by a sleep route" : "")
                + ")" + released);
    }

    /**
     * Put a shutdown line in the durable ring as well as returning it.
     *
     * <p>RETURNING IT IS NOT ENOUGH, and that was the second defect in the first
     * cut. The caller hands the returned line to `ingest_note`, which queues a
     * `TunerEvent` and posts a drain onto the SLINT EVENT LOOP — a loop that,
     * on the teardown this method runs from, will not be scheduled again. The
     * one report of what the release actually did died with the process it was
     * describing.
     *
     * <p>So it goes where the other durable notes go, under the key the next
     * launch already prints as `last sleep:`. That reader exists, it already
     * prints one line per entry, and this IS the sleep note — written from the
     * callback that arrives instead of the broadcast that never does.
     */
    private static String note(String line) {
        append(KEY_LAST_SLEEP, line);
        return line;
    }

    /** The write half of {@link #onAppDestroyed}, separated so a failure to
     *  read the MCU can still record the safe answer. */
    private static void setRadioPlaying(boolean playing) {
        try {
            ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .edit().putBoolean(KEY_RADIO_PLAYING, playing).commit();
        } catch (Throwable t) {
            Log.w(TAG, "could not record the radio state: " + t);
        }
    }

    // ── THE HANDBACK USED TO ERASE THE FACT IT WAS SUPPOSED TO PRESERVE ──────
    //
    // MEASURED, 2026-09-17, and the log reads as a clean success right up until
    // the last line:
    //
    //     11:11:40  mcu_state=3: state observer, kernel: source→Android sent
    //     11:11:42  shutdown: FM not playing (mcu_current_source was 0)
    //     11:11:43  bound, come-forward on, but the radio was not playing
    //
    // FM WAS PLAYING. `releaseSource` tests ownership before it sends anything
    // and it sent, so the source was 4 at 11:11:40. Two seconds later the
    // shutdown hook read the MCU, found 0 — because the handback had just worked
    // — and recorded "not playing". The listener then declined to come forward,
    // correctly, on a fact that was false.
    //
    // So the two features cancelled each other out: the better the handback
    // worked, the more reliably the come-forward gate was told the radio had
    // been off. `radio_playing` could not be true on any cycle where the thing
    // it gates on had actually happened.
    //
    // THE FIX IS TO RECORD IT WHERE IT IS STILL TRUE — in the route that hands
    // back, which has just tested for FM and knows. #133's outcome A asks about
    // the state BEFORE the sleep — *"If the radio wasn't playing when the unit
    // went to sleep"* — and that is what this now measures, rather than what is
    // left after this app has finished tidying up.

    /**
     * How long a sleep-time sighting of FM stays good, in milliseconds.
     *
     * <p>BOUNDED BECAUSE {@code SLEEP_ACTIONS} INCLUDES {@code ACTION_SCREEN_OFF}.
     * An unbounded flag would let a screen blank with FM playing mark the process
     * for the rest of its life, so a driver who then turned the radio off and
     * closed the app would still be recorded as having left it playing — the
     * come-forward defect, re-entered by a side door.
     *
     * <p>THIRTY SECONDS against a measured two. The gap between the handback at
     * 11:11:40 and the shutdown read at 11:11:42 is the interval this has to
     * cover, and an order of magnitude over it is enough for a slower cycle
     * without being long enough to span a driver changing their mind.
     */
    private static final long FM_AT_SLEEP_WINDOW_MS = 30_000L;

    /** When a sleep route last saw FM as the MCU's source. See below. */
    private static volatile long fmAtSleepElapsed = Long.MIN_VALUE;

    /**
     * Record that a sleep route found FM playing, BEFORE it hands the source
     * back.
     *
     * <p>BOTH HALVES, because either one alone has a hole. The durable write is
     * for the cycle where this process is killed without a Destroy and the
     * shutdown hook never runs at all; the in-memory stamp is for the ordinary
     * cycle, where the hook DOES run two seconds later and would otherwise read
     * the MCU we have just changed and overwrite the durable one with false.
     *
     * <p>ELAPSED REALTIME, which counts through a suspend, rather than
     * {@code uptimeMillis}, which stops. A stamp that froze with the SoC would
     * read as fresh on the far side of an ignition cycle.
     */
    static synchronized void noteFmAtSleep() {
        fmAtSleepElapsed = SystemClock.elapsedRealtime();
        setRadioPlaying(true);
    }

    /**
     * Whether a sleep route saw FM recently enough to believe.
     *
     * <p>The lower guard is not superstition: {@code elapsedRealtime} is
     * monotonic, but a negative age would mean the stamp came from the future,
     * and answering "yes, recent" to that is the one reading that cannot be
     * right.
     */
    private static boolean fmSeenAtSleepRecently() {
        long stamped = fmAtSleepElapsed;
        if (stamped == Long.MIN_VALUE) {
            return false;
        }
        long age = SystemClock.elapsedRealtime() - stamped;
        return age >= 0 && age <= FM_AT_SLEEP_WINDOW_MS;
    }
}
