package com.ninthfreak.carnyx;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.util.Log;

/**
 * Hand the FM source back when the head unit goes to sleep, from a process that
 * may not exist yet.
 *
 * <h2>Why the runtime receiver is not enough</h2>
 *
 * <p>{@code NwdBridge.startSleepWatch} registers for the same two vendor actions
 * at run time, and that is the right thing to do while the app is alive. But a
 * RUNTIME receiver lives in the process that registered it, and the fault being
 * chased here is precisely that the process may already be gone: the MCU kills
 * apps on ACC-off, and nothing says it kills them after broadcasting rather than
 * before. A dead process hears nothing, releases nothing, and leaves no trace —
 * which is exactly what "Carnyx does not shut off the radio audio when the head
 * unit sleeps" looks like from the driver's seat.
 *
 * <p>A MANIFEST receiver has the platform start a process to deliver the
 * broadcast. It is the same asymmetry {@link WakeReceiver} is built on, in the
 * other direction, and the same reason it is in this source tree rather than in
 * {@code java/} — see that class for the class-loader rule in full.
 *
 * <p>THE PATTERN IS CONFIRMED ON THIS ROM, which is not usually something a
 * receiver can say before it has run. The stock-radio probe resolved
 * {@code com.nwd.ACTION_OS_WAKE_UP → rcv:com.ninthfreak.carfm,
 * rcv:com.ninthfreak.carnyx}: a manifest receiver of ours, for a vendor action,
 * listed by the package manager as a live handler. So vendor broadcasts do reach
 * manifest receivers here, and this one is declared exactly the same way.
 *
 * <h2>Both fire, and that is harmless</h2>
 *
 * <p>When the app IS alive, both this and the runtime watch hear the same
 * broadcast and both send the same source-change. The MCU is being asked twice
 * to do something it has already done — the request carries the source it should
 * land on, not a step — so the second is a no-op. Two identical requests is a
 * cheaper failure than a missed one.
 *
 * <h2>SCREEN_OFF is not here</h2>
 *
 * <p>{@code NwdBridge}'s watch also listens for {@code ACTION_SCREEN_OFF}, and
 * this one cannot: Android 8 took it off the implicit-broadcast allowlist, so a
 * manifest filter for it is never delivered. That half stays where it works, in
 * the runtime watch, and covers the case this one does not.
 *
 * <h2>What it does NOT do</h2>
 *
 * <p>It does not touch the binder. {@code NwdBridge.releaseSource} also calls
 * {@code setRadioBackServiceOn(false)} through the vendor's AIDL, and in a cold
 * process there is no binding and no time to make one — {@code bindService} is
 * asynchronous and this receiver has milliseconds. What is left is the ONE call
 * the source probe found that actually sticks, and it needs nothing but a
 * Context.
 *
 * <p>It does not consult the app's settings file either, because there is no app
 * to read it. The driver's switch is mirrored into shared preferences by
 * {@link CarnyxWake#setReleaseOnSleep} for this reason alone.
 */
public final class SleepReceiver extends BroadcastReceiver {
    private static final String TAG = "CarnyxSleep";

    /**
     * BOTH SPELLINGS, because nobody knows which this ROM sends.
     *
     * <p>{@code NwdBridge.SLEEP_ACTIONS} carries the same pair and the same
     * note: the only record of this action anywhere writes it UNQUALIFIED, and
     * this ROM uses two prefixes for its own broadcasts. A filter for an action
     * nothing sends registers cleanly and never fires, so the wrong guess is
     * invisible. The action travels with the note this leaves, so one ignition
     * cycle names the right one.
     */
    private static final String ACTION_ACCOFF = "com.nwd.ACTION_ACCOFF_UPDATE";
    private static final String ACTION_ACCOFF_QUALIFIED = "com.nwd.action.ACTION_ACCOFF_UPDATE";

    /** The one that sticks. See {@code NwdBridge.releaseSource}. */
    private static final String ACTION_CHANGE_SOURCE = "com.nwd.action.ACTION_REQUEST_CHANGE_SOURCE";

    /** Shared with {@link CarnyxWake} by name. Keep the two in step. */
    private static final String PREFS = "carnyx_wake";
    private static final String KEY_LAST_SLEEP = "last_sleep";
    private static final String KEY_RELEASE_ON_SLEEP = "release_on_sleep";

    @Override
    public void onReceive(Context context, Intent intent) {
        if (context == null || intent == null || intent.getAction() == null) {
            return;
        }
        String action = intent.getAction();
        if (!ACTION_ACCOFF.equals(action) && !ACTION_ACCOFF_QUALIFIED.equals(action)) {
            // Something else matched the filter. Nothing to do, and nothing to
            // note — a line per stray broadcast would drown the one that matters.
            return;
        }

        if (!releaseOnSleep(context)) {
            note(context, action + ": manifest receiver, release is off");
            return;
        }

        try {
            context.sendBroadcast(new Intent(ACTION_CHANGE_SOURCE)
                    .putExtra("extra_source_id", (byte) 0));
            note(context, action + ": manifest receiver, source→0 sent");
        } catch (Throwable t) {
            note(context, action + ": manifest receiver, source→0 FAILED: " + t);
        }
    }

    /**
     * The driver's switch, as {@link CarnyxWake#setReleaseOnSleep} last left it.
     *
     * <p>DEFAULTS TO TRUE, which is the opposite of {@code WakeReceiver}'s
     * choice about its own flag and for the opposite reason. There, acting on an
     * unreadable flag would take the screen from whatever the driver was
     * looking at. Here the failure is silence — the radio playing into a parked
     * car — and the setting's own default is on, so an unset value means a
     * driver who has never touched the switch rather than one who turned it off.
     */
    private boolean releaseOnSleep(Context context) {
        try {
            SharedPreferences p = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            return p.getBoolean(KEY_RELEASE_ON_SLEEP, true);
        } catch (Throwable t) {
            Log.w(TAG, "could not read the release switch: " + t);
            return true;
        }
    }

    /**
     * Leave one line for the app to find on its way up.
     *
     * <p>{@code commit()}, for {@link WakeReceiver#note}'s reason and one more:
     * the MCU is cutting power to the SoC and this app holds no wake lock, so
     * an {@code apply()} whose background thread is never scheduled loses the
     * one artefact this whole path can produce.
     *
     * <p>Written through the same key {@link CarnyxWake#noteSleep} uses, so the
     * app reads ONE note whichever receiver got there first. The last writer
     * wins, and when both fire they are saying the same thing.
     */
    private void note(Context context, String line) {
        Log.i(TAG, line);
        try {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .edit().putString(KEY_LAST_SLEEP, line).commit();
        } catch (Throwable t) {
            Log.w(TAG, "could not record the sleep note: " + t);
        }
    }
}
