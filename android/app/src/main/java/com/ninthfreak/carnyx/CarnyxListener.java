package com.ninthfreak.carnyx;

import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.service.notification.NotificationListenerService;
import android.util.Log;

/**
 * A notification listener that reads no notifications.
 *
 * <h2>Why this exists, which is not what it looks like</h2>
 *
 * <p>The head unit FORCE-STOPS third-party apps when it sleeps — measured, not
 * assumed: the keep-alive probe reported "stopped: 8 of 17 third-party packages"
 * and named the vendor cleaner. A force-stopped package receives NO broadcast of
 * any kind, exempt or not, until a human taps its icon. That single fact killed
 * the wake receiver, the sleep receiver and the runtime sleep watch, and it is
 * why an ignition cycle produces no line in the diagnostics log from any of
 * them.
 *
 * <p>A {@code NotificationListenerService} is bound BY THE PLATFORM rather than
 * by the app, and the platform re-binds it. That is the only property this class
 * is here for. It does not want notifications and never looks at one:
 * {@code onNotificationPosted} and {@code onNotificationRemoved} are left at
 * their inherited no-ops, deliberately and permanently.
 *
 * <h2>WHAT IS ACTUALLY UNKNOWN, and this class is the experiment</h2>
 *
 * <p>Whether the platform re-binds a listener belonging to a package the vendor
 * has force-stopped. The Android documentation is about the ordinary lifecycle;
 * the vendor cleaner is not ordinary, and no amount of reading settles it. So
 * {@link #onListenerConnected} writes a durable note BEFORE it does anything
 * else, and the next launch reads it back. One ignition cycle then says:
 *
 * <ul>
 *   <li>a {@code listener:} line naming a bind — the platform brought this
 *       process back with no human involved, which is the precondition for
 *       everything in #133's outcome C;
 *   <li>no line — the force-stop takes the listener down with everything else,
 *       and C needs a different mechanism or is not reachable at all.
 * </ul>
 *
 * <p>THE NOTE IS THE POINT even if the launch below never runs. This class earns
 * its place by answering the question either way.
 *
 * <h2>Coming forward, and why it ships OFF</h2>
 *
 * <p>#94 removed a switch called "Start radio on boot" because it described
 * behaviour the app did not have. The lesson taken there applies here: an app
 * that takes the screen on every platform bind, on a mechanism nobody has
 * watched work, is that same promise made again. So the launch is gated on a
 * flag that defaults to false and lives in the shared preferences file a COLD
 * PROCESS can read — this service may be bound with no app, no Rust and no
 * settings file parsed, exactly as {@code SleepReceiver} may be.
 *
 * <p>Its hazard is real and worth stating: a bind is not only a wake. The
 * platform binds at boot, after the driver grants the permission, and after its
 * own rebind timer. With the flag on, each of those brings the face forward,
 * and one of them could be while the driver is deliberately in maps.
 *
 * <h2>Privacy</h2>
 *
 * <p>Holding this permission means the platform WOULD hand over the content of
 * every notification on the unit. This class overrides neither callback, keeps
 * no state about them, and has no code path that reads one. If that ever stops
 * being true, it stops being true in a commit that says so.
 */
public final class CarnyxListener extends NotificationListenerService {

    private static final String TAG = "CarnyxListener";

    /**
     * Shared with {@link CarnyxWake} BY NAME, the way {@code SleepReceiver}
     * shares it. The two halves can never meet in memory: when the platform
     * binds this service there may be no Rust loaded, so a constant cannot be
     * imported from the dexed side of the app.
     */
    private static final String PREFS = "carnyx_wake";

    /** What the last bind or unbind did. Read back by {@code takeLastListener}. */
    private static final String KEY_LAST_LISTENER = "last_listener";

    /** The driver's switch. False, and this service only writes its note. */
    private static final String KEY_COME_FORWARD = "come_forward";

    @Override
    public void onListenerConnected() {
        // WRITTEN FIRST, before the flag is read and before anything can throw.
        // This line is the experiment; the launch below is the feature, and a
        // feature that fails must not cost the evidence.
        note("bound by the platform");

        boolean forward;
        try {
            forward = getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .getBoolean(KEY_COME_FORWARD, false);
        } catch (Throwable t) {
            note("bound, but the come-forward flag could not be read: " + t);
            return;
        }
        if (!forward) {
            return;
        }

        try {
            Intent launch = getPackageManager().getLaunchIntentForPackage(getPackageName());
            if (launch == null) {
                note("bound, come-forward on, but this package has no launch intent");
                return;
            }
            // As WakeReceiver: a service has no task of its own to start an
            // activity into, and the platform throws without the flag.
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            startActivity(launch);
            note("bound, and brought the face forward");
        } catch (Throwable t) {
            // The likely one is Android 10's background-activity-start refusal.
            // Nothing to recover; say which, so it is diagnosable rather than
            // indistinguishable from a bind that never happened.
            note("bound, but the launch was refused: " + t);
        }
    }

    @Override
    public void onListenerDisconnected() {
        note("unbound by the platform");
    }

    /**
     * One durable line, overwritten each time.
     *
     * <p>{@code commit()} and not {@code apply()}, for {@code WakeReceiver}'s
     * reason: this may be running in a process the platform is about to tear
     * down, and an {@code apply()} whose background thread never got scheduled
     * would lose the evidence this class exists to produce.
     */
    private void note(String line) {
        Log.i(TAG, line);
        try {
            SharedPreferences p = getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            p.edit().putString(KEY_LAST_LISTENER, line).commit();
        } catch (Throwable t) {
            Log.w(TAG, "could not record the listener note: " + t);
        }
    }
}
