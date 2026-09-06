package com.ninthfreak.carnyx;

import android.content.Context;
import android.content.SharedPreferences;
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
