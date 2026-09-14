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

    /** The driver's switch. False, and this service only writes its note. */
    private static final String KEY_COME_FORWARD = "come_forward";

    /**
     * Whether FM was the MCU's source when this app last shut down.
     *
     * <p>THE SECOND GATE, AND THE ONE THE FIRST BUILD LACKED. See
     * {@code CarnyxWake.KEY_RADIO_PLAYING} for what writes it and why it is
     * measured off the MCU rather than believed. The switch above says whether
     * the driver WANTS the face back; this says whether there is anything to come
     * back for. #133's outcome A keeps the unit's own condition — *"If the radio
     * wasn't playing when the unit went to sleep, the stock radio app doesn't get
     * launched"* — and a build that ignored it put the face on screen after
     * ignition cycles in which nothing had been playing.
     *
     * <p>DEFAULT FALSE. A key that is absent means this app has never recorded a
     * clean shutdown, and silence is the right answer to a question nobody can
     * answer: a face that fails to appear is a disappointment, and a face that
     * appears over a driver's map is the defect.
     *
     * <p>AND IT IS SPENT WHEN IT IS READ. It describes ONE shutdown, while
     * {@link #onListenerConnected} fires on every rebind the platform decides to
     * make — so a flag left standing is a face that comes back over whatever the
     * driver is using, again and again. See the consume in that method.
     */
    private static final String KEY_RADIO_PLAYING = "radio_playing";

    @Override
    public void onListenerConnected() {
        // WRITTEN FIRST, before the flag is read and before anything can throw.
        // This line is the experiment; the launch below is the feature, and a
        // feature that fails must not cost the evidence.
        note("bound by the platform");

        boolean forward;
        boolean playing;
        try {
            SharedPreferences p =
                    getSharedPreferences(CarnyxNotes.PREFS, Context.MODE_PRIVATE);
            forward = p.getBoolean(KEY_COME_FORWARD, false);
            playing = p.getBoolean(KEY_RADIO_PLAYING, false);
        } catch (Throwable t) {
            note("bound, but the come-forward flags could not be read: " + t);
            return;
        }
        if (!forward) {
            return;
        }
        // THE CONDITION THE UNIT ITSELF APPLIES, kept rather than discarded. The
        // vendor resumes its radio app only when FM was playing into the sleep;
        // #133's outcome A asks for Carnyx in its place, WITH that condition
        // intact. Noted rather than silent, because "did nothing, and here is
        // why" is the line that separates a gate from a failure.
        if (!playing) {
            note("bound, come-forward on, but the radio was not playing at shutdown");
            return;
        }

        // ── SPENT ON READING, BEFORE THE LAUNCH AND NOT AFTER IT ─────────────
        //
        // THE OWNER: *"The app likes to keep itself in front of everything.
        // Everything. Even when I want to use a different app, it keeps
        // reasserting itself."* This is that defect. `radio_playing` describes
        // ONE shutdown, and `onListenerConnected` does not fire once — the
        // platform rebinds a notification listener on package changes, on
        // settings changes, on its own rebind timer, and every time this process
        // is killed and started again. A flag that stays true turns each of
        // those into another `startActivity`, and the driver who is trying to
        // use maps gets the radio back every time.
        //
        // So the fact is CONSUMED, the way every other durable note in this tree
        // is consumed — `CarnyxWake.take` clears on read for exactly this
        // reason. One shutdown grants one attempt to come forward.
        //
        // BEFORE THE LAUNCH, so that a refusal spends it too. Android 10 can
        // refuse a background activity start; if that left the flag set, every
        // subsequent rebind would retry, which is the same loop by another road.
        // One attempt is what was asked for, not one success.
        try {
            getSharedPreferences(CarnyxNotes.PREFS, Context.MODE_PRIVATE)
                    .edit().putBoolean(KEY_RADIO_PLAYING, false).commit();
        } catch (Throwable t) {
            // A flag that cannot be cleared is the loop above waiting to happen,
            // so this does NOT go on to launch. Silence is recoverable by the
            // driver; a face that will not stay out of the way is not.
            note("bound, but the come-forward flag could not be spent: " + t);
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
     * One entry in the listener's ring. See {@link CarnyxNotes}, which owns the
     * file name, the cap and the blocking write.
     */
    private void note(String line) {
        CarnyxNotes.append(this, CarnyxNotes.KEY_LAST_LISTENER, line);
    }
}
