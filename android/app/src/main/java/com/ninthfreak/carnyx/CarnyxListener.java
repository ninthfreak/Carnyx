package com.ninthfreak.carnyx;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.SharedPreferences;
import android.database.ContentObserver;
import android.os.Build;
import android.provider.Settings;
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

        // BEFORE EVERY EARLY RETURN BELOW. The come-forward gate has four of
        // them, and the sleep watch has nothing to do with whether the face
        // comes forward — a driver with come-forward off still wants the radio
        // to stop at ACC-off.
        startSleepWatch();

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

    // ── THE COLD SLEEP WATCH ────────────────────────────────────────────────
    //
    // WHY IT MOVED HERE FROM `SleepReceiver`, which is the component that looks
    // like it already does this job.
    //
    // `SleepReceiver` is a MANIFEST receiver, and this app sets `targetSdk = 34`.
    // Android 8 stopped delivering IMPLICIT broadcasts to manifest receivers in
    // any app targeting 26 or above, and the vendor sends its state change with a
    // plain one-argument `sendBroadcast` — no package, no component, no flags, so
    // implicit. A manifest filter for it registers cleanly, resolves cleanly, and
    // is never delivered. `SleepReceiver`'s own class doc cites the stock-radio
    // probe listing it as a live handler for a vendor action, but the package
    // manager RESOLVES a filter while the broadcast queue DISPATCHES it, and the
    // restriction lives in the queue. Resolution was never the evidence it read
    // as.
    //
    // A RUNTIME receiver has no such restriction, and neither does a
    // ContentObserver. This service is the right process to hold them: the
    // platform binds a NotificationListenerService itself and re-binds it after
    // the vendor force-stop, which is the whole reason this class exists.
    //
    // WHAT THIS PATH CANNOT DO. `CarnyxKernel` — the synchronous binder call
    // that reaches the UART before it returns — lives in `java/`, dexed into the
    // app's own class loader, and this file is loaded by the platform. The two
    // halves cannot meet in memory. So the cold path sends the BROADCAST only,
    // the route measured to lose the race at ACC-off. It is worth having anyway
    // for the case it is the only route there is: a sleep that arrives with no
    // app process, where the alternative is nothing at all.

    /** The vendor's ACC transition announcement. Matches `NwdBridge` by name. */
    private static final String MCU_STATE_ACTION = "com.nwd.action.ACTION_MCU_STATE_CHANGE";

    /** 1 awake, 2 and 3 going down, 0 powered off. Read, not trusted from extras. */
    private static final String MCU_STATE_KEY = "mcu_state";

    /** The MCU's current audio source. 4 is FM. */
    private static final String MCU_SOURCE_KEY = "mcu_current_source";

    /** The one call the source probe found that sticks. See `NwdBridge`. */
    private static final String ACTION_CHANGE_SOURCE = "com.nwd.action.ACTION_REQUEST_CHANGE_SOURCE";

    /** The driver's switch, as {@link CarnyxWake#setReleaseOnSleep} left it. */
    private static final String KEY_RELEASE_ON_SLEEP = "release_on_sleep";

    /**
     * Registered once per process and never torn down.
     *
     * <p>Static because the platform may bind a NEW instance of this service
     * without the old process having died, and two watches would send the source
     * change twice. There is no unregister for `NwdBridge`'s reason: this watch
     * COMMANDS the MCU, and the moment it is needed is the moment nothing else in
     * this app is alive to do it.
     */
    private static boolean sleepWatched;

    /** Both routes to one fact. See {@link #startSleepWatch}. */
    private static BroadcastReceiver sleepReceiver;

    /** Both routes to one fact. See {@link #startSleepWatch}. */
    private static ContentObserver stateObserver;

    /**
     * Watch for the unit going to sleep, from the process the platform re-binds.
     *
     * <p>TWO MECHANISMS FOR ONE EVENT, matching {@code NwdBridge.startSleepWatch}
     * and for its reason: the broadcast is the vendor's nudge and the setting is
     * the vendor's fact, and whether a third-party app may receive that broadcast
     * is an assumption this app has already been wrong about three builds
     * running. Whichever arrives first does the work; the loser finds FM is no
     * longer the source and sends nothing.
     */
    private void startSleepWatch() {
        // ON THE CLASS, NOT THE INSTANCE. The flag is static because the guard has
        // to hold across INSTANCES — the platform can bind a new service object in
        // a process that already has a watch running — and a `synchronized` method
        // would lock `this`, which is a different lock for each of them and no
        // guard at all.
        synchronized (CarnyxListener.class) {
            if (sleepWatched) {
                return;
            }
            sleepWatched = true;
        }

        BroadcastReceiver r = new BroadcastReceiver() {
            @Override public void onReceive(Context c, Intent i) {
                onSleepSignal("broadcast");
            }
        };
        try {
            IntentFilter f = new IntentFilter(MCU_STATE_ACTION);
            if (Build.VERSION.SDK_INT >= 33) {
                registerReceiver(r, f, Context.RECEIVER_EXPORTED);
            } else {
                registerReceiver(r, f);
            }
            sleepReceiver = r;
            note("sleep watch: listening for " + MCU_STATE_ACTION);
        } catch (Throwable t) {
            note("sleep watch: broadcast registration failed: " + t);
        }

        try {
            // NULL HANDLER, so onChange runs on the binder thread that delivered
            // it rather than being posted to a looper the suspending SoC may
            // never schedule again. Same choice as `NwdBridge`, same reason.
            ContentObserver o = new ContentObserver(null) {
                @Override public void onChange(boolean selfChange) {
                    onSleepSignal("observer");
                }
            };
            getContentResolver().registerContentObserver(
                    Settings.System.getUriFor(MCU_STATE_KEY), false, o);
            stateObserver = o;
            note("sleep watch: watching " + MCU_STATE_KEY);
        } catch (Throwable t) {
            note("sleep watch: " + MCU_STATE_KEY + " watch failed: " + t);
        }
    }

    /**
     * One sleep signal, from either route.
     *
     * <p>GATED AT 2 AND 3, which is the vendor's own test — {@code AWRadioManager}
     * re-reads {@code mcu_state} on this broadcast and calls {@code ExitFm()} only
     * for those. Everything else is a wake or an unrelated write, and acting on
     * one would take the radio away at the instant the driver started the car.
     *
     * <p>SILENT ON A NON-SLEEP. Both routes fire on every ACC transition, and a
     * line per wake would push the sleep line out of a ring that keeps eight.
     */
    private void onSleepSignal(String route) {
        // READ ONCE. Two calls to `mcuInt` here would be two reads of a value the
        // MCU is actively changing, and the pair could straddle a transition.
        int state = mcuInt(MCU_STATE_KEY, 1);
        if (state != 2 && state != 3) {
            return;
        }
        // THE STATE TRAVELS WITH EVERY LINE BELOW. Which of 2 and 3 this ROM
        // actually sends is not established — the vendor treats them alike and so
        // does this — and the first log that carries one settles it.
        String where = "sleep watch (" + route + ", mcu_state=" + state + ")";
        if (!releaseOnSleep()) {
            note(where + ": release is off");
            return;
        }
        // ONLY RELEASE WHAT THIS UNIT IS ACTUALLY ON. source→0 is a command to
        // the MCU and not a request to give back something we hold, so sending it
        // while Bluetooth audio is the source would switch away from whatever was
        // playing. `NwdBridge.releaseSource` makes the same test first.
        int src = mcuInt(MCU_SOURCE_KEY, -1);
        if (src != 4) {
            // NOTHING IS WRITTEN ON THIS BRANCH, and the restraint is the whole
            // reason this gate is safe. Seeing a source that is not FM here does
            // NOT mean FM was off: the app's own routes run in another process,
            // fire on the same signal, and may have handed the source back
            // milliseconds ago. Writing `radio_playing = false` from here would
            // be this process clobbering a true the app had just recorded
            // correctly — the 2026-09-17 defect, re-created across the process
            // boundary. The flag is cleared by the shutdown hook and spent on
            // read below; this route only ever adds a sighting.
            note(where + ": FM is not the source (" + src + ")");
            return;
        }
        // RECORDED BEFORE THE RELEASE. See `CarnyxWake.noteFmAtSleep`: after the
        // next line this fact stops being observable, and this may be the only
        // process still alive to observe it.
        try {
            getSharedPreferences(CarnyxNotes.PREFS, Context.MODE_PRIVATE)
                    .edit().putBoolean(KEY_RADIO_PLAYING, true).commit();
        } catch (Throwable t) {
            note(where + ": FM was playing but the flag could not be recorded: " + t);
        }
        try {
            sendBroadcast(new Intent(ACTION_CHANGE_SOURCE)
                    .putExtra("extra_source_id", (byte) 0));
            note(where + ": source→0 sent");
        } catch (Throwable t) {
            note(where + ": source→0 FAILED: " + t);
        }
    }

    /** One {@code Settings.System} integer, or the fallback. Never throws. */
    private int mcuInt(String key, int fallback) {
        try {
            String v = Settings.System.getString(getContentResolver(), key);
            return v == null ? fallback : Integer.parseInt(v.trim());
        } catch (Throwable t) {
            return fallback;
        }
    }

    /**
     * The driver's switch.
     *
     * <p>DEFAULTS TO TRUE, for {@code SleepReceiver.releaseOnSleep}'s reason: the
     * failure here is silence — the radio playing into a parked car — and the
     * setting's own default is on, so an unset value means a driver who has never
     * touched the switch rather than one who turned it off.
     */
    private boolean releaseOnSleep() {
        try {
            return getSharedPreferences(CarnyxNotes.PREFS, Context.MODE_PRIVATE)
                    .getBoolean(KEY_RELEASE_ON_SLEEP, true);
        } catch (Throwable t) {
            Log.w(TAG, "could not read the release switch: " + t);
            return true;
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
