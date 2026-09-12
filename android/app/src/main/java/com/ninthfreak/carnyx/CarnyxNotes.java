package com.ninthfreak.carnyx;

import android.content.Context;
import android.content.SharedPreferences;
import android.os.SystemClock;
import android.util.Log;

/**
 * The durable notes the manifest components leave for the next launch.
 *
 * <h2>Why this class exists</h2>
 *
 * <p>Three components write these — {@code WakeReceiver}, {@code SleepReceiver}
 * and {@code CarnyxListener} — and each carried its own private {@code note()}
 * with its own copy of the preferences file name. Three copies of six lines,
 * which is three places for a fix to miss. One now.
 *
 * <p>IT CANNOT COVER {@code CarnyxWake}, and that is not an oversight. That class
 * is in {@code java/}, dexed by {@code build.rs} and loaded at run time by an
 * {@code InMemoryDexClassLoader}; these three are in the Gradle source set and
 * are constructed by the platform through the APPLICATION's class loader, which
 * has never heard of the dexed ones. The two halves can never meet in memory, so
 * the file name and the keys are shared BY NAME across that divide and the
 * appending rule below is stated twice. That split is already documented on
 * {@code CarnyxWake.PREFS}; this is the same divide, not a new one.
 *
 * <h2>A RING, NOT A SLOT, and what that fixes</h2>
 *
 * <p>Each note used to be one value, overwritten. The reader takes and clears —
 * so the note appeared in the log of the FIRST launch after the event and was
 * erased at that moment. If that session died before the driver exported, the
 * evidence was gone for good: the diagnostics log is a ring in memory and does
 * not survive the process either.
 *
 * <p>On a unit where the whole experiment is "switch the car off, switch it on,
 * and see what was recorded", that put the answer one mis-step from being lost.
 * Now each write APPENDS, up to {@link #KEEP} entries, and the reader drains all
 * of them. Several ignition cycles can pass, and several launches, and the notes
 * still arrive together.
 *
 * <p>OLDEST GOES FIRST when the cap is reached, which is the right end to drop:
 * these are read after the fact, and the most recent cycle is the one being
 * asked about.
 */
final class CarnyxNotes {

    private static final String TAG = "CarnyxNotes";

    /** Shared with {@code CarnyxWake} by name. See the class note. */
    static final String PREFS = "carnyx_wake";

    static final String KEY_LAST_WAKE = "last_wake";
    static final String KEY_LAST_SLEEP = "last_sleep";
    static final String KEY_LAST_LISTENER = "last_listener";

    /**
     * How many entries one key keeps.
     *
     * <p>Eight, because the thing being measured is an ignition cycle and nobody
     * drives eight of them between exports — and because these are read into a
     * log whose head holds {@code DiagLog::HEAD_CAP} lines in total. A cap that
     * outran the head would push the rest of the head into the scrolling ring,
     * which is the part that does not survive.
     */
    static final int KEEP = 8;

    /** Entries are joined by this. Nothing written here contains one. */
    static final String SEP = "\n";

    private CarnyxNotes() {}

    /**
     * Append one line to a key's ring.
     *
     * <p>{@code commit()} rather than {@code apply()}: every caller may be
     * running in a process the platform or the MCU is about to tear down, and an
     * {@code apply()} whose background thread never got scheduled would lose
     * exactly the evidence this exists to produce. Read-modify-write under that
     * same blocking call — two components writing the same key in the same
     * millisecond could lose one entry, which is a trade worth taking over a
     * lock a dying process might not release.
     */
    static void append(Context ctx, String key, String line) {
        Log.i(TAG, key + ": " + line);
        if (ctx == null || line == null || line.isEmpty()) {
            return;
        }
        line = stamp() + "  " + line;
        try {
            SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            String prev = p.getString(key, "");
            String next = prev == null || prev.isEmpty() ? line : prev + SEP + line;
            p.edit().putString(key, trim(next)).commit();
        } catch (Throwable t) {
            Log.w(TAG, "could not record a " + key + " note: " + t);
        }
    }

    /**
     * What every entry is stamped with: the wall clock, and how long this unit
     * has been asleep since it last booted.
     *
     * <h2>THE STAMP IS THE MEASUREMENT, not decoration</h2>
     *
     * <p>An undated "bound by the platform" is four different events wearing the
     * same words: the driver granted the permission, the unit booted, the platform
     * re-bound on its own timer, or THE UNIT WOKE. Only the last one answers #133,
     * and the log these end up in stamps its lines with the time each note was
     * READ, not written — so a note from an hour ago and one from a second ago
     * printed identically. That is exactly what left the 2026-09-08 drive
     * undecided.
     *
     * <p>THE SLEPT FIGURE IS THE HALF THAT CANNOT BE FAKED. Android keeps two
     * counters since boot: one that keeps running while the unit is suspended and
     * one that stops. Their difference is how long this unit has spent asleep in
     * total. It never decreases, so comparing it between two notes says whether a
     * SLEEP happened between them — the one event the wall clock cannot show,
     * because the clock advances the same either way.
     *
     * <p>So two notes reading {@code slept 41m} and {@code slept 58m} bracket a
     * seventeen-minute sleep, and a bind stamped on the far side of that gap is
     * the platform starting this app after the unit woke. Nothing in this app has
     * ever been able to say that.
     *
     * <p>Neither reading can realistically throw; both are guarded anyway, because
     * a note that fails to record is the single failure this class exists to
     * prevent.
     */
    static String stamp() {
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
            // COMPUTED BEFORE ANYTHING IS APPENDED, which is not style. Appending
            // " slept " first and then computing left `" slept  slept ?"` in the
            // ring whenever the clock read threw — the buffer already carried the
            // label the catch was about to add again. The other copy of this
            // method, across the class-loader divide in `CarnyxWake`, computed
            // first and so read correctly; two rings that disagree on the same
            // input are exactly what stating the rule twice is supposed to avoid.
            String slept = forHumans(SystemClock.elapsedRealtime() - SystemClock.uptimeMillis());
            b.append(" slept ").append(slept);
        } catch (Throwable t) {
            b.append(" slept ?");
        }
        return b.toString();
    }

    /**
     * Milliseconds as {@code 41m} or {@code 3h12m}.
     *
     * <p>MINUTES ARE THE FLOOR, deliberately: this is read to spot a GAP between
     * two notes, and seconds of resolution on a figure that grows by whole
     * ignition cycles would be noise on a line read from a dashboard.
     */
    static String forHumans(long ms) {
        long m = ms / 60000L;
        return m < 60 ? m + "m" : (m / 60) + "h" + (m % 60) + "m";
    }

    /**
     * Drop the oldest entries until at most {@link #KEEP} remain.
     *
     * <p>Package-private and static so {@code CarnyxWake} cannot share it but a
     * test could reach it; there is no test harness on this side of the divide
     * yet, which is itself worth knowing.
     */
    static String trim(String joined) {
        String[] parts = joined.split(SEP, -1);
        if (parts.length <= KEEP) {
            return joined;
        }
        StringBuilder b = new StringBuilder();
        for (int i = parts.length - KEEP; i < parts.length; i++) {
            if (b.length() > 0) {
                b.append(SEP);
            }
            b.append(parts[i]);
        }
        return b.toString();
    }
}
