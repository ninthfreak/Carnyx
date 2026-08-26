package com.ninthfreak.carnyx;

import android.content.BroadcastReceiver;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.ServiceConnection;
import android.content.res.Configuration;
import android.media.AudioManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.provider.Settings;
import android.util.Log;

import com.nwd.radio.service.RadioCallback;
import com.nwd.radio.service.RadioFeature;
import com.nwd.radio.service.data.Frequency;
import com.nwd.radio.service.data.RadioPoint;

/**
 * The Android edge of Carnyx's tuner. Binder in, JNI out — and NOTHING ELSE.
 *
 * WHY THERE IS JAVA IN A PURE-RUST APP AT ALL. The NWD tuner is an AIDL binder
 * service. There is no NDK path to binder, and cargo-apk packages no Java: this
 * class is compiled by build.rs, dexed, embedded in the .so with include_bytes!
 * and loaded at runtime with InMemoryDexClassLoader. The mechanism is copied
 * from Slint's own android-activity backend, which ships it on this device.
 *
 * WHAT THIS CLASS IS ALLOWED TO DECIDE: nothing. It reads the vendor's raw
 * integers and strings and hands them to Rust unaltered — the frequency SCALE is
 * not applied here, the band is not tracked here, no string is formatted here.
 * That rule is what makes the tuner logic testable in a container with no head
 * unit: every value that reaches Rust from a real device is a value the fake can
 * produce too. The one exception is stated where it happens (the level watch
 * gates itself on the MCU source, because a wrong tick COMMANDS the tuner and a
 * round trip to Rust for permission would not make it safer).
 *
 * PORTED FROM CarFM's android/app/src/main/java/com/ninthfreak/carfm/
 * NwdRadioModule.kt, which is device-proven. The comments that record what the
 * device taught are carried across with the code — they are the evidence for
 * every non-obvious choice below, and they cost more to re-learn than to keep.
 *
 * STILL NOT PORTED: the raw-RDS capture-to-file and its Downloads export, and
 * the two long diagnostic dumps (probeNwdFmManager and the full getter walk).
 *
 * THAT LIST USED TO SAY "deliberately", AND THAT WAS THE WRONG CALL. The
 * standing instruction for this port is that everything CarFM had which did not
 * come from VibeSDR gets ported; judging these to be diagnostics worth skipping
 * was not a judgement to make here. `writeLog` below is the first of them back,
 * and it is the one that mattered most — without it the diagnostics log could
 * only leave this unit as a screenshot of its last few lines, on a unit with no
 * adb, while the log ring holds 200. The rest are owed, and docs/TASKS.md #87
 * carries them with what each one needs.
 *
 * THREADING. Vendor callbacks arrive on binder threads; the RDS pump and level
 * watch each own a daemon thread. All four can call into Rust concurrently, so
 * the Rust side takes a lock. Nothing here touches the UI.
 */
public final class NwdBridge {

    private static final String TAG = "CarnyxNwd";

    /** The bind Intent and the package that answers it. Both verified on device. */
    private static final String BIND_ACTION = "com.nwd.radio.service.ACTION_RADIO_SERVICE";
    private static final String SERVICE_PKG = "com.nwd.radio.service";

    /**
     * How long to wait for onServiceConnected before calling the bind dead.
     *
     * A successful bindService only means the system accepted the request;
     * onServiceConnected is the event that matters, and if the vendor service
     * dies in onCreate or hands back a null binder it never arrives. Without
     * this, CarFM sat forever inside `await nwdConnect()` — above every line
     * that clears the tuner-error pill — leaving a dead face with no way back
     * short of killing the app.
     */
    private static final long CONNECT_TIMEOUT_MS = 8000L;

    private NwdBridge() {}

    // ── Process-wide state ────────────────────────────────────────────────────
    // Static, because there is exactly one tuner in one process and passing an
    // instance handle across JNI would buy nothing. The context is the
    // APPLICATION context (see attach) so holding it statically leaks nothing.
    private static Context ctx;
    private static RadioFeature radio;
    private static boolean bound;
    private static boolean registered;
    private static final Object LOCK = new Object();
    private static Handler mainHandler;

    /**
     * Guards the connect result so it is reported exactly once, whoever gets
     * there first: onServiceConnected arrives on the main thread while connect()
     * runs on the caller's thread, and the watchdog races both.
     */
    private static boolean connectPending;

    // ── Attach ────────────────────────────────────────────────────────────────

    /**
     * Hand the bridge the app context. Called once from Rust at start-up, with
     * the NativeActivity; getApplicationContext() is taken so nothing here can
     * outlive and leak an Activity.
     */
    public static synchronized void attach(Context activity) {
        if (ctx == null && activity != null) {
            ctx = activity.getApplicationContext();
            mainHandler = new Handler(Looper.getMainLooper());
            Log.i(TAG, "attached");
        }
    }

    // ── Detection ─────────────────────────────────────────────────────────────

    /**
     * True if this unit exposes the NWD radio service. Cheap; safe to call any
     * time.
     *
     * ON API 30+ THIS NEEDS A MANIFEST &lt;queries&gt; ENTRY for
     * com.nwd.radio.service, or package-visibility filtering makes resolveService
     * return null on a unit that plainly has the service — and bindService fails
     * the same way. See the note in Cargo.toml.
     */
    public static boolean isAvailable() {
        try {
            Intent intent = new Intent(BIND_ACTION).setPackage(SERVICE_PKG);
            return ctx != null && ctx.getPackageManager().resolveService(intent, 0) != null;
        } catch (Throwable t) {
            return false;
        }
    }

    // ── Binding lifecycle ─────────────────────────────────────────────────────

    /**
     * Request the bind. Returns false when the request itself was refused; the
     * OUTCOME arrives asynchronously as nativeConnected or nativeConnectFailed.
     *
     * Idempotent, and that is load-bearing: Carnyx starts the tuner as early as
     * it can reach it (CarFM's warm start) and the screen connects again when it
     * mounts. The second call must resolve against the live binding rather than
     * bind twice.
     */
    public static boolean connect() {
        synchronized (LOCK) {
            if (bound && radio != null) {
                reportConnected();
                return true;
            }
            if (connectPending) {
                return true;
            }
            connectPending = true;
        }
        if (ctx == null) {
            settleFailed("connect() before attach()");
            return false;
        }
        try {
            Intent intent = new Intent(BIND_ACTION).setPackage(SERVICE_PKG);
            boolean ok = ctx.bindService(intent, CONN, Context.BIND_AUTO_CREATE);
            if (!ok) {
                try { ctx.unbindService(CONN); } catch (Throwable ignored) {}
                settleFailed("bindService returned false — NWD radio service not found/bindable");
                return false;
            }
            synchronized (LOCK) { bound = true; }
            startPanelKeyWatch();
            startIlluminationWatch();
            startSleepWatch();
            // NOT startRdsPump() here: bindService is asynchronous, so at this
            // point `radio` is still null and setRDSState has not run. The pump
            // starts from onServiceConnected, once RDS has actually been enabled.
            mainHandler.postDelayed(new Runnable() {
                @Override public void run() {
                    if (takePending()) {
                        Log.w(TAG, "connect timed out after " + CONNECT_TIMEOUT_MS + "ms");
                        disconnect();
                        safeConnectFailed("NWD radio service bound but never connected ("
                                + CONNECT_TIMEOUT_MS + "ms)");
                    }
                }
            }, CONNECT_TIMEOUT_MS);
            return true;
        } catch (Throwable t) {
            settleFailed("bindService threw: " + t.getMessage());
            return false;
        }
    }

    public static void disconnect() {
        if (mainHandler != null) mainHandler.removeCallbacksAndMessages(null);
        // A disconnect while a connect is still in flight must not leave the
        // caller waiting on a result that now has nothing left to produce it.
        if (takePending()) {
            safeConnectFailed("disconnected before the NWD radio service connected");
        }
        stopPanelKeyWatch();
        // The ILLUMINATION watch deliberately survives a disconnect. Day/night
        // is a property of the vehicle, not of which tuner is selected, and
        // CarFM tore it down here — so a session that dropped the built-in tuner
        // (or never took it) stopped hearing the headlights. There is nothing to
        // stop it for: one unregister at process death is the whole cost.
        stopRdsPump();
        stopLevelWatch();
        RadioFeature r;
        boolean wasBound;
        synchronized (LOCK) {
            r = radio;
            wasBound = bound;
            radio = null;
            bound = false;
            registered = false;
        }
        if (r != null) {
            try { r.unRegistCallback(CALLBACK); } catch (Throwable ignored) {}
        }
        if (wasBound && ctx != null) {
            try { ctx.unbindService(CONN); } catch (Throwable ignored) {}
        }
    }

    private static boolean takePending() {
        synchronized (LOCK) {
            boolean was = connectPending;
            connectPending = false;
            return was;
        }
    }

    private static void settleFailed(String reason) {
        takePending();
        safeConnectFailed(reason);
    }

    private static final ServiceConnection CONN = new ServiceConnection() {
        @Override public void onServiceConnected(ComponentName name, IBinder binder) {
            RadioFeature r = RadioFeature.Stub.asInterface(binder);
            boolean reg;
            try {
                r.registCallback(CALLBACK);
                reg = true;
            } catch (Throwable t) {
                Log.w(TAG, "registCallback failed", t);
                reg = false;
            }
            synchronized (LOCK) {
                radio = r;
                registered = reg;
            }
            // RDS on by default (selector byte 0 — the value the CarFM spike
            // confirmed works on this firmware).
            try { r.setRDSState((byte) 0, true); } catch (Throwable ignored) {}
            // Only now is the raw-group transport worth polling: the service is
            // live and RDS has actually been switched on.
            startRdsPump();
            takePending();
            reportConnected();
        }

        @Override public void onServiceDisconnected(ComponentName name) {
            synchronized (LOCK) { radio = null; }
            // The pump reads through NwdFmManager, not the binder, so it would
            // keep polling a dead front end. Stop it; a re-bind re-arms it.
            stopRdsPump();
            stopLevelWatch();
            safeDisconnected();
        }
    };

    /**
     * Report the connected state to Rust.
     *
     * NO INITIAL STEREO READ. isStreroOn() is stuck true on this firmware (it
     * reads true on dead air), so seeding from it put a false STEREO on CarFM's
     * face that nothing corrected. The notifyStereo callback is the only
     * trustworthy source.
     *
     * The frequency is passed RAW. Deciding what scale it is in is Rust's job —
     * see Calibration in src/android/mod.rs.
     */
    private static void reportConnected() {
        RadioFeature r;
        boolean reg;
        synchronized (LOCK) { r = radio; reg = registered; }
        int band = 0;
        int raw = 0;
        String ps = "";
        if (r != null) {
            try {
                Frequency f = r.getCurrentFrequency();
                if (f != null) {
                    band = f.band;
                    raw = f.freq;
                    ps = f.psName == null ? "" : f.psName;
                }
            } catch (Throwable t) {
                Log.w(TAG, "getCurrentFrequency failed", t);
            }
        }
        String rt = "";
        int pty = -1;
        if (r != null) {
            try { rt = r.getRtMessage(); } catch (Throwable ignored) {}
            try { pty = r.getPTYType(); } catch (Throwable ignored) {}
        }
        safeConnected(band, raw, ps, rt == null ? "" : rt, pty, reg);
    }

    // ── Control ───────────────────────────────────────────────────────────────

    /**
     * Tune. The frequency is the tuner's OWN raw integer and the band its own
     * byte — Rust computed both, because the MHz-to-raw multiplier is a property
     * of the unit and is learned at run time.
     */
    public static boolean tune(int rawFreq, int band) {
        RadioFeature r;
        synchronized (LOCK) { r = radio; }
        if (r == null) return false;
        try {
            r.setCurrentFrequency(rawFreq, (byte) band, 0);
            return true;
        } catch (Throwable t) {
            Log.w(TAG, "tune failed", t);
            return false;
        }
    }

    /**
     * Hardware seek to the next receivable station.
     *
     * Uses search(), NOT seek(): the CarFM probe of 2026-07-26 showed
     * radio.seek() only nudges one 0.2 MHz step and lands on noise, while
     * radio.search() scans and STOPS on the next real station in both directions
     * (88.7 to 91.1 and back, clear audio). search() is the genuine on-demand
     * seek and works outside the preset auto-search.
     */
    public static void seek(boolean up) {
        RadioFeature r;
        synchronized (LOCK) { r = radio; }
        if (r == null) return;
        try { r.search(up); } catch (Throwable t) { Log.w(TAG, "seek (search) failed", t); }
    }

    public static void setRdsEnabled(boolean on) {
        RadioFeature r;
        synchronized (LOCK) { r = radio; }
        if (r == null) return;
        try { r.setRDSState((byte) 0, on); } catch (Throwable t) { Log.w(TAG, "setRDSState failed", t); }
    }

    /**
     * Claim / release the FM audio source. The tuner's audio is analog and
     * MCU-routed, so producing sound means BECOMING the active source, and
     * stopping it means the MCU actually LETTING GO — setRadioBackServiceOn
     * alone does neither.
     *
     * ON  -> announce we are the FM source the way the probe proved works
     *        (ACTION_APP_IN_OUT app_id=8, operation=1 IN), which drives the
     *        service's InitFM/powerUp and makes the MCU route tuner audio
     *        (mcu_current_source becomes 4); keep the back-service alive and
     *        unmute.
     * OFF -> switch the MCU source AWAY from Radio
     *        (ACTION_REQUEST_CHANGE_SOURCE extra_source_id=0). This is the ONLY
     *        thing that sticks: the probe of 2026-07-26 proved EXIT_ARM_FM and
     *        app-OUT (operation=0) both left mcu_current_source=4 and the MCU
     *        re-powered FM a second later — the "comes back on" bug — while
     *        source->0 made the audio STAY off.
     */
    public static void setAudioEnabled(boolean on) {
        if (ctx == null) return;
        RadioFeature r;
        synchronized (LOCK) { r = radio; }
        if (on) {
            sendAppInOut(1);
            if (r != null) {
                try { r.setRadioBackServiceOn(true); } catch (Throwable t) { Log.w(TAG, "backservice on failed", t); }
            }
            try {
                AudioManager am = (AudioManager) ctx.getSystemService(Context.AUDIO_SERVICE);
                if (am != null) am.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_UNMUTE, 0);
            } catch (Throwable ignored) {}
        } else {
            releaseSource();
        }
    }

    /**
     * Hand the FM source back, and say what happened.
     *
     * <p>SPLIT OUT OF {@link #setAudioEnabled} SO THE SLEEP RECEIVER CAN CALL IT
     * ON ITS OWN THREAD. Going through Rust means `ingest_sleep` -> `emit` ->
     * the event queue -> `invoke_from_event_loop` -> the Slint event loop ->
     * `drain_events`, which is a thread hop and a drain taken at the exact
     * moment the MCU has announced it is cutting power to the SoC. This app
     * holds no wake lock — neither manifest declares WAKE_LOCK — so nothing
     * guarantees the loop is scheduled again before the suspend. The release has
     * to be issued on the thread that heard the broadcast.
     *
     * <p>The queued release still runs afterwards and is harmless: this is a
     * re-send of the same two calls, and it is what the host tests drive.
     *
     * @return what it managed, for the diagnostics log. Never null.
     */
    static synchronized String releaseSource() {
        if (ctx == null) {
            return "no context";
        }
        RadioFeature r;
        synchronized (LOCK) { r = radio; }
        String back = "no binder";
        if (r != null) {
            try {
                r.setRadioBackServiceOn(false);
                back = "backservice off";
            } catch (Throwable t) {
                back = "backservice off failed: " + why(t);
            }
        }
        try {
            // THE ONLY ONE THE PROBE FOUND THAT STICKS. See the note above
            // setAudioEnabled: EXIT_ARM_FM and app-OUT both left
            // mcu_current_source at 4.
            ctx.sendBroadcast(new Intent("com.nwd.action.ACTION_REQUEST_CHANGE_SOURCE")
                    .putExtra("extra_source_id", (byte) 0));
            return "source→0 sent, " + back;
        } catch (Throwable t) {
            return "source→0 FAILED: " + why(t) + ", " + back;
        }
    }

    /**
     * A throwable as one short line: the simple name, the message, and the cause
     * if there is one, truncated so a single failure cannot fill the ring.
     *
     * <p>Every catch that reaches a person goes through this. A bare
     * {@code getSimpleName()} names a class and not a reason —
     * "(RuntimeException)" is a dead end where
     * "RuntimeException: ... &lt;- TransactionTooLargeException" names a remedy.
     */
    static String why(Throwable t) {
        if (t == null) {
            return "unknown";
        }
        StringBuilder b = new StringBuilder(t.getClass().getSimpleName());
        if (t.getMessage() != null && !t.getMessage().isEmpty()) {
            b.append(": ").append(t.getMessage());
        }
        Throwable cause = t.getCause();
        if (cause != null && cause != t) {
            b.append(" <- ").append(cause.getClass().getSimpleName());
        }
        return b.length() > 120 ? b.substring(0, 120) + "…" : b.toString();
    }

    /**
     * ACTION_APP_IN_OUT app_id=8 — the source claim/release the stock app uses
     * and the probe validated. operation: 1 = app IN (claim), 0 = app OUT.
     */
    private static void sendAppInOut(int operation) {
        try {
            ctx.sendBroadcast(new Intent("com.nwd.action.ACTION_APP_IN_OUT")
                    .putExtra("extra_app_id", 8)
                    .putExtra("extra_app_operation", operation)
                    .putExtra("extra_app_event", 0));
        } catch (Throwable ignored) {}
    }

    // ── Polling ───────────────────────────────────────────────────────────────

    /**
     * The tuner's current state, as raw numbers.
     *
     * [0] raw frequency, [1] band, [2] PTY, [3] stereo getter (0/1), [4] MCU
     * source. Null when not connected. Index 3 is the STUCK getter — it reads
     * true on dead air — and is reported only so Rust can say so; the stereo
     * pill must follow the notifyStereo event instead.
     *
     * On-device the push notify* callbacks do not always reach a passive client,
     * but these synchronous getters do return live values (proven by the
     * connect-time seed), which is why the face polls at all.
     *
     * [4] is the MCU's own source register: 4 = FM, -1 = unreadable. Android
     * audio focus cannot answer "is FM actually playing" on a head unit — the
     * audio is analog and MCU-routed, and after a permanent AUDIOFOCUS_LOSS the
     * OS never sends a GAIN, so a focus-driven state can go dark and never come
     * back. The MCU register is the truth and it self-heals.
     */
    public static int[] pollNumbers() {
        RadioFeature r;
        synchronized (LOCK) { r = radio; }
        if (r == null) return null;
        int[] out = new int[] { 0, 0, -1, 0, -1 };
        try {
            Frequency f = r.getCurrentFrequency();
            if (f != null) { out[0] = f.freq; out[1] = f.band; }
        } catch (Throwable ignored) {}
        try { out[2] = r.getPTYType(); } catch (Throwable ignored) {}
        try { out[3] = r.isStreroOn() ? 1 : 0; } catch (Throwable ignored) {}
        out[4] = mcuSource();
        return out;
    }

    public static String pollPs() {
        RadioFeature r;
        synchronized (LOCK) { r = radio; }
        if (r == null) return "";
        try {
            Frequency f = r.getCurrentFrequency();
            return (f == null || f.psName == null) ? "" : f.psName;
        } catch (Throwable t) {
            return "";
        }
    }

    public static String pollRt() {
        RadioFeature r;
        synchronized (LOCK) { r = radio; }
        if (r == null) return "";
        try {
            String rt = r.getRtMessage();
            return rt == null ? "" : rt;
        } catch (Throwable t) {
            return "";
        }
    }

    /**
     * The band plan, flattened to lo, hi, step per band. Empty when unreadable.
     * Units differ per band (FM in 10 kHz, AM in 1 kHz) so these stay raw and
     * Rust scales them — see BandPoint in src/android/mod.rs.
     */
    public static int[] bandPlan() {
        RadioFeature r;
        synchronized (LOCK) { r = radio; }
        if (r == null) return new int[0];
        try {
            RadioPoint[] arr = r.getRadioPoint();
            if (arr == null) return new int[0];
            int[] out = new int[arr.length * 3];
            for (int i = 0; i < arr.length; i++) {
                out[i * 3] = arr[i].lo;
                out[i * 3 + 1] = arr[i].hi;
                out[i * 3 + 2] = arr[i].step;
            }
            return out;
        } catch (Throwable t) {
            return new int[0];
        }
    }

    /** Read the MCU's current audio source. 4 = FM; -1 = could not read. */
    private static int mcuSource() {
        try {
            String v = Settings.System.getString(ctx.getContentResolver(), "mcu_current_source");
            return v == null ? -1 : Integer.parseInt(v.trim());
        } catch (Throwable t) {
            return -1;
        }
    }

    // ── Steering-wheel / panel keys (the REAL transport) ───────────────────────
    // The wheel is NOT a media key on this unit. The MCU broadcasts
    // com.nwd.action.ACTION_KEY_VALUE with a byte extra_key_value, and the vendor
    // RadioService registers for it WITHOUT a permission, then routes it through
    // its own dispatch table — gated on Settings.System mcu_current_source == 4
    // (FM). That is why MediaSession capture and Activity.dispatchKeyEvent both
    // saw nothing in CarFM: the event never enters Android's input pipeline.
    //
    // Because the receiver is unprotected, we can listen for the same broadcast.
    // The code-to-meaning table lives in Rust (PanelKey in src/android/mod.rs);
    // this only forwards the number.
    //
    // Note that keys 62/63 make the SERVICE step its own hardware preset bank. A
    // normal broadcast cannot be cancelled, so the service still acts; the app
    // reasserts its own preset immediately after, which is what makes the app's
    // order win.
    //
    // THAT LAST CLAUSE DESCRIBED AN INTENTION, NOT THE CODE, for as long as it
    // has been here. The app tuned once and stopped, so whichever of the two
    // commands reached the front end last was the station that played — and when
    // it was the vendor's, nothing put the driver back: "a few times when using
    // the steering wheel controls, it did end up jumping to a wrong station."
    // It is true now. State::reassert in src/app.rs is what makes it so, and
    // examples/wheelprobe.rs case F is what measures it.
    private static BroadcastReceiver panelReceiver;

    private static synchronized void startPanelKeyWatch() {
        if (panelReceiver != null || ctx == null) return;
        BroadcastReceiver r = new BroadcastReceiver() {
            @Override public void onReceive(Context c, Intent i) {
                if (i == null || i.getAction() == null) return;
                String a = i.getAction();
                int key;
                if ("com.nwd.action.ACTION_KEY_VALUE".equals(a)) {
                    key = i.getByteExtra("extra_key_value", (byte) -1);
                } else if ("com.nwd.action.ACTION_TEST_KEY".equals(a)) {
                    key = i.getIntExtra("extra_key_value", -1);
                } else {
                    return;
                }
                if (key < 0) return;
                safePanelKey(key, a);
            }
        };
        IntentFilter f = new IntentFilter();
        f.addAction("com.nwd.action.ACTION_KEY_VALUE");
        f.addAction("com.nwd.action.ACTION_TEST_KEY");
        if (registerExported(r, f)) {
            panelReceiver = r;
            Log.i(TAG, "panel-key watch registered");
        }
    }

    private static synchronized void stopPanelKeyWatch() {
        if (panelReceiver != null && ctx != null) {
            try { ctx.unregisterReceiver(panelReceiver); } catch (Throwable ignored) {}
        }
        panelReceiver = null;
    }

    /**
     * Send a panel key AS IF the wheel/panel had been pressed (the same
     * unprotected broadcast the MCU uses), so the service's own dispatch table
     * can be exercised without physical buttons.
     */
    public static void sendPanelKey(int key) {
        if (ctx == null) return;
        try {
            ctx.sendBroadcast(new Intent("com.nwd.action.ACTION_KEY_VALUE")
                    .putExtra("extra_key_value", (byte) key));
        } catch (Throwable ignored) {}
    }

    // ── The diagnostics log, out to Downloads ─────────────────────────────────
    //
    // PORTED FROM CarFM's VibeStreamModule.writeLog
    // (android/app/src/main/java/com/ninthfreak/carfm/VibeStreamModule.kt:715-745),
    // where it is device-proven, and its comments are carried across because they
    // are the evidence for the two non-obvious choices.
    //
    // NO SAF AND NO PICKER ACTIVITY. CarFM's note: the SAF picker crashed on units
    // with no DocumentsUI, and this head unit is one of them. Downloads is a
    // standard location a file manager can see and copy to a USB stick, which is
    // the only way anything leaves this unit — it has no adb.
    //
    // API 29+ goes through MediaStore and needs no permission at all. Below that
    // the public Downloads directory is written directly, which needs
    // WRITE_EXTERNAL_STORAGE; the manifest declares it with maxSdkVersion="32",
    // exactly as CarFM's does, so it is requested only where it is used.

    /**
     * Write {@code text} to the PUBLIC Downloads folder and return a human path.
     *
     * <p>THE RETURN VALUE CARRIES THE FAILURE, prefixed with "!". A thrown
     * exception would reach Rust as a bare {@code Error::JavaException} with the
     * message left pending on the JVM, and the message is the whole point: this
     * unit has no adb and no logcat a driver can read, so the only place a reason
     * can be shown is the diagnostics panel that asked for the save.
     */
    public static String writeLog(String text) {
        try {
            if (ctx == null) return "!no context";
            String stamp = new java.text.SimpleDateFormat("yyyyMMdd-HHmmss", java.util.Locale.US)
                    .format(new java.util.Date());
            String fileName = "carnyx-tuner-log-" + stamp + ".txt";
            byte[] bytes = text.getBytes(java.nio.charset.StandardCharsets.UTF_8);
            if (android.os.Build.VERSION.SDK_INT >= 29) {
                android.content.ContentResolver resolver = ctx.getContentResolver();
                android.content.ContentValues values = new android.content.ContentValues();
                values.put(android.provider.MediaStore.Downloads.DISPLAY_NAME, fileName);
                values.put(android.provider.MediaStore.Downloads.MIME_TYPE, "text/plain");
                values.put(android.provider.MediaStore.Downloads.RELATIVE_PATH,
                        android.os.Environment.DIRECTORY_DOWNLOADS);
                android.net.Uri uri = resolver.insert(
                        android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
                if (uri == null) throw new java.io.IOException("MediaStore insert returned null");
                java.io.OutputStream out = resolver.openOutputStream(uri);
                if (out == null) throw new java.io.IOException("openOutputStream returned null");
                try {
                    out.write(bytes);
                } finally {
                    out.close();
                }
                return "Downloads/" + fileName;
            }
            java.io.File dir = android.os.Environment.getExternalStoragePublicDirectory(
                    android.os.Environment.DIRECTORY_DOWNLOADS);
            dir.mkdirs();
            java.io.File f = new java.io.File(dir, fileName);
            java.io.FileOutputStream out = new java.io.FileOutputStream(f);
            try {
                out.write(bytes);
            } finally {
                out.close();
            }
            return f.getAbsolutePath();
        } catch (Throwable e) {
            String why = e.getMessage();
            return "!" + (why == null || why.isEmpty() ? e.toString() : why);
        }
    }

    // ── Illumination (headlights) -> day/night ────────────────────────────────
    // Every other app on this unit switches to dark when the headlights go on.
    // The vendor broadcast is com.nwd.ACTION_ILL_STATE_CHANGE carrying
    // extra_ill_state, and the extra's TYPE is unknown (byte / int / boolean) —
    // so this dumps the whole bundle verbatim rather than guessing a getter and
    // reading a silent default. It also reports Android's uiMode night bits at
    // the same instant, which is the decisive comparison:
    //   ill fires AND uiMode flips  -> the ROM does set night mode
    //   ill fires, uiMode unchanged -> vendor-only signal; we must listen for it
    //
    // Day/night is a property of the VEHICLE, not of which tuner is selected, so
    // this is deliberately callable without connecting the tuner. Idempotent.
    private static BroadcastReceiver illReceiver;

    public static synchronized void startIlluminationWatch() {
        if (illReceiver != null || ctx == null) return;
        BroadcastReceiver r = new BroadcastReceiver() {
            @Override public void onReceive(Context c, Intent i) {
                Bundle extras = i == null ? null : i.getExtras();
                StringBuilder dump = new StringBuilder();
                if (extras == null) {
                    dump.append("(no extras)");
                } else {
                    boolean first = true;
                    for (String k : extras.keySet()) {
                        if (!first) dump.append(", ");
                        first = false;
                        Object v = extras.get(k);
                        dump.append(k).append('=').append(v).append(" (")
                            .append(v == null ? "null" : v.getClass().getSimpleName()).append(')');
                    }
                }
                safeIllumination(i == null || i.getAction() == null ? "" : i.getAction(),
                        dump.toString(), uiModeNight());
            }
        };
        IntentFilter f = new IntentFilter();
        f.addAction("com.nwd.ACTION_ILL_STATE_CHANGE");
        if (registerExported(r, f)) {
            illReceiver = r;
            Log.i(TAG, "ill watch registered; uiMode=" + uiModeNight());
        }
    }

    private static BroadcastReceiver sleepReceiver;

    /**
     * The driver's "Release FM on sleep" switch, MIRRORED FROM RUST.
     *
     * <p>The receiver above runs on a binder thread with no route into the app's
     * state — `Settings::release_on_sleep` lives behind a `RefCell` on the UI
     * thread — and it has to honour the switch, because releasing for a driver
     * who turned it off is exactly the SCREEN_OFF hazard the switch exists to
     * let them avoid. So the value is pushed down the same way
     * `CarnyxWake.setForeground` pushes the foreground flag.
     *
     * <p>Volatile, not synchronized: it is read on a binder thread and written
     * on the UI thread, and a stale read costs one ignition cycle of the old
     * behaviour. TRUE by default, matching `Settings::default`, so a build where
     * the mirror never runs still releases.
     */
    private static volatile boolean releaseOnSleep = true;

    /**
     * See {@link #releaseOnSleep}. Called from Rust whenever the switch moves.
     *
     * <p>MIRRORED TO DISK AS WELL, because this field only reaches the runtime
     * watch. {@code SleepReceiver} is a manifest component that runs in a cold
     * process with no app, no settings file read and no Rust — a static here
     * says nothing to it. {@link CarnyxWake#setReleaseOnSleep} is the copy it
     * can read.
     */
    public static void setReleaseOnSleep(boolean on) {
        releaseOnSleep = on;
        CarnyxWake.setReleaseOnSleep(on);
    }

    /**
     * Watch for the head unit going to sleep, so the FM source can be handed back
     * before this process stops running.
     *
     * <p>WHY: the MCU sleeps the SoC on ACC-off, remembers the current source
     * across the sleep, and restores it on ACC-on — so a unit left on FM comes
     * back into FM and the stock radio app launches itself. Handing the source
     * back on the way down leaves nothing for it to restore.
     *
     * <p>TWO ACTIONS, NOT EQUALLY GOOD.
     *
     * <p>{@code com.nwd.ACTION_ACCOFF_UPDATE} is the precise one — CarFM's
     * BootReceiver records the vendor service handling it on the way down, which
     * is exactly the moment wanted. What is NOT known is whether a third-party
     * app receives it at all: it is a vendor action, it may be protected, and
     * nothing in either app has ever listened for it. If it never arrives this
     * costs nothing and the other action still fires.
     *
     * <p>{@code ACTION_SCREEN_OFF} is the certain one — a standard system
     * broadcast, and unlike the activity lifecycle it does not fire when the
     * driver merely switches to maps. ITS HAZARD IS REAL: a screen timeout with
     * the engine running looks identical from here, and the audio would stop with
     * the driver still listening. There is no auto-reclaim, because reclaiming
     * would put the source back and undo the point of releasing it, so recovery
     * is the power button. Delete the one {@code addAction} to remove that
     * half.
     *
     * <p>SCREEN_OFF CANNOT BE A MANIFEST RECEIVER (Android 8 took it off the
     * implicit-broadcast allowlist), which is no obstacle: this app is alive to
     * register it, and the foreground service is what keeps it that way.
     */
    /**
     * The actions this watch listens for.
     *
     * <p>TWO SPELLINGS OF THE VENDOR ACTION, because NOBODY KNOWS WHICH IS RIGHT.
     * The only record of it anywhere is CarFM's BootReceiver comment — "the
     * vendor service handles {@code ACTION_ACCOFF_UPDATE} going down" — and its
     * handoff's broadcast list, and BOTH write it UNQUALIFIED where they write
     * the other two out in full. This ROM uses two prefixes: {@code com.nwd.} for
     * {@code ACTION_OS_WAKE_UP} and {@code ACTION_ILL_STATE_CHANGE},
     * {@code com.nwd.action.} for {@code ACTION_KEY_VALUE},
     * {@code ACTION_REQUEST_CHANGE_SOURCE} and {@code ACTION_APP_IN_OUT}. The
     * first cut of this file picked {@code com.nwd.} by analogy with the wake
     * broadcast, which was a COIN FLIP recorded as a fact.
     *
     * <p>A filter for an action nothing ever sends registers cleanly and never
     * fires, so the wrong guess is invisible — which is what "Carnyx does not
     * shut off the radio audio when the head unit sleeps" would look like. Both
     * spellings cost one line each, and the action travels with the event, so
     * ONE ignition cycle names the right one in the diagnostics log.
     *
     * <p>SCREEN_OFF is the certain one and its hazard is on the switch that turns
     * this feature on; see the class note below.
     */
    private static final String[] SLEEP_ACTIONS = {
        "com.nwd.ACTION_ACCOFF_UPDATE",
        "com.nwd.action.ACTION_ACCOFF_UPDATE",
        Intent.ACTION_SCREEN_OFF,
    };

    /**
     * @return a line for the DIAGNOSTICS LOG saying what happened, never null.
     *     It used to return void and say so to logcat, which on a unit with no
     *     adb reaches nobody — so "the receiver never registered" and "the
     *     broadcast never arrived" looked identical from the driver's seat, and
     *     they need different fixes.
     */
    public static synchronized String startSleepWatch() {
        if (sleepReceiver != null) {
            return "already registered";
        }
        if (ctx == null) {
            return "no context — attach() has not run";
        }
        BroadcastReceiver r = new BroadcastReceiver() {
            @Override public void onReceive(Context c, Intent i) {
                String action = i == null || i.getAction() == null ? "" : i.getAction();
                // RELEASED HERE, ON THIS THREAD, BEFORE THE HOP TO RUST. See
                // releaseSource: the queued path is a thread hop and a drain
                // taken while the MCU is cutting power, with no wake lock behind
                // it. The outcome travels with the event so the diagnostics log
                // records what this call managed rather than what it attempted.
                String outcome = releaseOnSleep ? releaseSource() : "skipped, release is off";
                // WRITTEN DOWN BEFORE IT IS LOGGED, and the ordering is the
                // point. `safeSleep` hops to Rust and queues a line into a ring
                // that lives IN MEMORY and dies with this process — which the
                // MCU has just announced it is about to kill. Every sleep line
                // this app has written was lost that way, which is why the
                // owner's report that the radio does not shut off at sleep could
                // not be answered from the log at all: the run that recorded it
                // was gone before anyone could read it. This lands on disk, with
                // a blocking write, on the thread that heard the broadcast. See
                // CarnyxWake.noteSleep.
                CarnyxWake.noteSleep(action + ": runtime watch, " + outcome);
                safeSleep(action, outcome);
            }
        };
        IntentFilter f = new IntentFilter();
        StringBuilder names = new StringBuilder();
        for (String a : SLEEP_ACTIONS) {
            f.addAction(a);
            if (names.length() > 0) {
                names.append(", ");
            }
            names.append(a);
        }
        if (!registerExported(r, f)) {
            return "registerReceiver REFUSED — nothing will be heard";
        }
        sleepReceiver = r;
        Log.i(TAG, "sleep watch registered");
        return "listening for " + names;
    }

    // There is no stopIlluminationWatch, on purpose. See disconnect().

    /** Android's night flag right now, as a readable string. */
    private static String uiModeNight() {
        try {
            int mode = ctx.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK;
            if (mode == Configuration.UI_MODE_NIGHT_YES) return "NIGHT";
            if (mode == Configuration.UI_MODE_NIGHT_NO) return "DAY";
        } catch (Throwable ignored) {}
        return "UNDEFINED";
    }

    /**
     * Register a receiver for a broadcast that comes from ANOTHER app (the MCU
     * service). On API 33+ that requires the RECEIVER_EXPORTED flag or the
     * registration throws.
     */
    private static boolean registerExported(BroadcastReceiver r, IntentFilter f) {
        try {
            if (Build.VERSION.SDK_INT >= 33) {
                ctx.registerReceiver(r, f, Context.RECEIVER_EXPORTED);
            } else {
                ctx.registerReceiver(r, f);
            }
            return true;
        } catch (Throwable t) {
            Log.w(TAG, "registerReceiver failed", t);
            return false;
        }
    }

    // ── The vendor framework class ────────────────────────────────────────────
    //
    // The bound AIDL does NOT carry raw RDS or a signal level on this unit. The
    // transport that does is the vendor framework class com.nwd.app.NwdFmManager,
    // reached by reflection (it is a ROM addition, so it is not on the
    // hidden-API blocklist, which is generated from AOSP).
    //
    // The android.os.Hardware JSON channel that the findings doc discusses is
    // ABSENT here — it belongs to the Si4792x variant of this firmware and this
    // unit runs the Allwinner path. Do not re-add speculative parseJson calls.
    //
    // ONLY THREE NAMES ARE REACHABLE FROM HERE, and that is structural rather
    // than a convention. CarFM once had a general int-argument reflection helper
    // with the comment "still a getter: nothing here commands the tuner". That
    // was wrong and it cost the audio: sweeping getRadioRDSStrengthArm over args
    // 0..3 cut the sound instantly and returned 63 (0x3F) for every one. "get"
    // in this API does not mean read-only, so there is no general helper here to
    // invite the next plausible-looking name.
    private static final String FM_MANAGER = "com.nwd.app.NwdFmManager";

    private static Object fmManagerGet(String name) throws Exception {
        Class<?> cls = Class.forName(FM_MANAGER);
        java.lang.reflect.Method m;
        try {
            m = cls.getMethod(name);
        } catch (NoSuchMethodException e) {
            m = cls.getDeclaredMethod(name);
            m.setAccessible(true);
        }
        return m.invoke(null);
    }

    // ── Raw RDS pump ──────────────────────────────────────────────────────────
    // getRadioRDSDataArm() returns ONE already-synchronised RDS group as 16 hex
    // chars (4 blocks x 2 bytes). "Nothing this poll" comes back as all zeros OR
    // as a Java null — both observed on device, so both are skipped. This is the
    // data the bound AIDL never exposes, and the reason RadioText is reachable
    // on this unit at all. Confirmed live 2026-08-01.
    //
    // RDS runs at ~11.4 groups/sec, so ~87ms per group; polling at 90ms keeps up
    // without spinning.
    //
    // NOTHING IS DEDUPLICATED HERE. CarFM dropped a group identical to the
    // previous one to save bridge traffic, and its own later comment records
    // that as a mistake: the decoder's consensus gates COUNT repeats (PI needs
    // 3, PTY 3, PS two agreeing assemblies), so dropping them starves exactly
    // the thing they feed. There is no bridge to save here — this is a direct
    // JNI call — so the whole stream goes across.
    private static final long RDS_POLL_MS = 90L;
    private static final String ZERO_GROUP = "0000000000000000";

    // Arming budget: 240 x 250ms = 60s. The gate reads a hardware capability
    // flag, so it only has to outlast the vendor stack coming up — it is not
    // waiting on a station. Bounded so a unit without the transport at all does
    // not keep a thread alive forever.
    private static final int RDS_ARM_ATTEMPTS = 240;
    private static final long RDS_ARM_RETRY_MS = 250L;

    private static volatile boolean rdsRunning;
    private static Thread rdsThread;

    private static synchronized void startRdsPump() {
        if (rdsThread != null) return;
        rdsRunning = true;
        Thread t = new Thread(new Runnable() {
            @Override public void run() {
                // The support check runs INSIDE the thread, with retries, and
                // asks only whether the HARDWARE has RDS — never whether a
                // station is sending it.
                //
                // It used to be a one-shot gate evaluated by the caller, which
                // ran before the service existed: launch while the head unit was
                // on Bluetooth and it latched supported=false for the whole
                // session, on the only path that can produce RadioText.
                //
                // The gate also once demanded a 16-char data read, which is a
                // property of the STATION, not the tuner: on 2026-08-03 a quiet
                // channel read (null) eight times out of eight while the
                // capability flag still read 1. So the FLAG alone arms the pump;
                // a null data read is just a poll with nothing in it.
                boolean armed = false;
                int attempts = 0;
                while (rdsRunning) {
                    if (!armed) {
                        boolean ok;
                        try {
                            Object v = fmManagerGet("getRadioRDSFunArm");
                            ok = (v instanceof Integer) && ((Integer) v) == 1;
                        } catch (Throwable t) {
                            ok = false;
                        }
                        if (ok) {
                            armed = true;
                            Log.i(TAG, "RDS pump armed after " + attempts + " attempt(s)");
                        } else if (++attempts >= RDS_ARM_ATTEMPTS) {
                            Log.i(TAG, "RDS pump giving up: transport unavailable");
                            break;
                        } else {
                            try { Thread.sleep(RDS_ARM_RETRY_MS); } catch (InterruptedException e) { break; }
                            continue;
                        }
                    }
                    String hex;
                    try {
                        Object v = fmManagerGet("getRadioRDSDataArm");
                        hex = v == null ? "" : v.toString();
                    } catch (Throwable t) {
                        hex = "";
                    }
                    if (hex.length() == 16 && !ZERO_GROUP.equals(hex)) {
                        safeRdsGroup(hex);
                    }
                    try { Thread.sleep(RDS_POLL_MS); } catch (InterruptedException e) { break; }
                }
            }
        }, "carnyx-rds-pump");
        t.setDaemon(true);
        t.start();
        rdsThread = t;
        Log.i(TAG, "RDS pump thread started (" + RDS_POLL_MS + "ms)");
    }

    private static synchronized void stopRdsPump() {
        rdsRunning = false;
        if (rdsThread != null) rdsThread.interrupt();
        rdsThread = null;
    }

    // ── Signal level ──────────────────────────────────────────────────────────
    //
    // WHAT THIS DOES, and why this argument and no other. From CarFM's decompile
    // of the vendor service (2026-08-03):
    //
    //   AWNative.seek(int frequency) {
    //       int packed   = NwdFmManager.seek(frequency);
    //       int strength = getFreAndStrength(packed, 1);   // high 4 hex digits
    //       int freq     = getFreAndStrength(packed, 0);   // low  4 hex digits
    //       return (freq == frequency) ? strength : 0;     // 0 = it moved
    //   }
    //
    // and the vendor's own alternative-frequency follower ends by calling
    // AWNative.seek(currentFrequency) as a no-op restore. So the vendor itself
    // calls seek with the frequency it is ALREADY ON, and the equality check
    // exists to confirm the tuner stayed put. That is the call made here: one
    // invocation, at the current raw frequency.
    //
    // IT IS STILL A COMMAND, not a read. Hence: its own thread, never a shared
    // one; a hard floor on the interval; and it stops the moment FM is not the
    // MCU's current source, so it can never retune a front end that Bluetooth or
    // Android Auto is using. The vendor rate-limits its own comparable read to
    // 900ms; this runs far below that.
    private static final long LEVEL_MIN_INTERVAL_MS = 5000L;

    private static volatile boolean levelRunning;
    private static Thread levelThread;

    /**
     * Restart guard. A bare boolean races: stop sets it false, start sets it
     * true again, and if the outgoing thread has not re-checked in between it
     * simply carries on, leaving two threads commanding the tuner. Each thread
     * captures a generation and exits when it is no longer the current one.
     */
    private static volatile int levelGen;

    /** One seek, serialized, so two callers can never command the tuner at once. */
    private static final Object SEEK_LOCK = new Object();

    public static void startLevelWatch(long intervalMs) {
        stopLevelWatch();               // idempotent, and the only way to change cadence
        final int myGen;
        synchronized (LOCK) { myGen = ++levelGen; }
        levelRunning = true;
        final long gap = Math.max(LEVEL_MIN_INTERVAL_MS, intervalMs);
        Thread t = new Thread(new Runnable() {
            @Override public void run() {
                while (levelRunning && levelGen == myGen) {
                    if (mcuSource() == 4) seekHere();
                    try { Thread.sleep(gap); } catch (InterruptedException e) { break; }
                }
            }
        }, "carnyx-level-watch");
        t.setDaemon(true);
        t.start();
        levelThread = t;
        Log.i(TAG, "level watch started (" + gap + "ms)");
    }

    public static void stopLevelWatch() {
        levelRunning = false;
        synchronized (LOCK) { levelGen++; }
        if (levelThread != null) levelThread.interrupt();
        levelThread = null;
    }

    /**
     * One reading now, off the caller's thread — used on retune so the meter
     * does not sit on the previous station's level until the next tick.
     */
    public static void readLevelNow() {
        Thread t = new Thread(new Runnable() {
            @Override public void run() { seekHere(); }
        }, "carnyx-level-once");
        t.setDaemon(true);
        t.start();
    }

    /**
     * Ask the tuner for its level at the frequency it is ALREADY ON, and report
     * it.
     *
     * Reads the frequency live from the binder every time — never from cached
     * state, because a stale number would turn this into a real retune, which is
     * the one thing it must not be.
     */
    private static void seekHere() {
        synchronized (SEEK_LOCK) {
            RadioFeature r;
            synchronized (LOCK) { r = radio; }
            if (r == null) { safeLevel(0, 0, 0, false, "not connected"); return; }
            int asked;
            try {
                Frequency f = r.getCurrentFrequency();
                if (f == null) { safeLevel(0, 0, 0, false, "getCurrentFrequency returned null"); return; }
                asked = f.freq;
            } catch (Throwable t) {
                safeLevel(0, 0, 0, false, "getCurrentFrequency threw " + t.getClass().getSimpleName());
                return;
            }
            if (asked <= 0) { safeLevel(0, asked, 0, false, "current frequency reads " + asked); return; }
            Object packed;
            try {
                Class<?> cls = Class.forName(FM_MANAGER);
                java.lang.reflect.Method m;
                try {
                    m = cls.getMethod("seek", int.class);
                } catch (NoSuchMethodException e) {
                    m = cls.getDeclaredMethod("seek", int.class);
                    m.setAccessible(true);
                }
                packed = m.invoke(null, asked);
            } catch (Throwable t) {
                Throwable cause = (t instanceof java.lang.reflect.InvocationTargetException)
                        ? ((java.lang.reflect.InvocationTargetException) t).getCause() : t;
                if (cause == null) cause = t;
                safeLevel(0, asked, 0, false,
                        "seek threw " + cause.getClass().getName() + ": " + cause.getMessage());
                return;
            }
            if (!(packed instanceof Integer)) {
                safeLevel(0, asked, 0, false, "seek returned "
                        + (packed == null ? "null" : packed.getClass().getName()) + ", not Int");
                return;
            }
            int p = (Integer) packed;
            int strength = (p >>> 16) & 0xFFFF;
            int landed = p & 0xFFFF;
            // Whether the reading is TRUSTWORTHY is the same test AWNative makes
            // before it believes a level, but it is a decision — so it is made in
            // Rust from asked and landed, not here.
            safeLevel(strength, asked, landed, true, null);
        }
    }

    // ── Vendor callbacks -> Rust ──────────────────────────────────────────────
    private static final RadioCallback.Stub CALLBACK = new RadioCallback.Stub() {
        @Override public void notifyState(byte s) { safeRadioState(s); }

        @Override public void notifyCurrentFrequency(byte band, int freq, String ps, int arg) {
            // Everything here goes across RAW. The band must be TRACKED (each
            // band has its own preset bank and its own tuning), and the MHz
            // scale must be RE-DERIVED on every notification rather than
            // calibrated once — CarFM learned both the hard way, and both
            // decisions now live in Calibration on the Rust side where they can
            // be tested. `arg` is the tuner's preset-slot index in the current
            // bank (1-6), or -1 when the tuned frequency is not a preset there;
            // it is NOT a signal level.
            safeFrequency(band, freq, ps == null ? "" : ps, arg);
        }

        @Override public void notifyNearOn(boolean on) {}

        @Override public void notifyStereo(boolean on) { safeStereo(on); }

        @Override public void notifyStereoOn(boolean on) { safeStereo(on); }

        @Override public void notifyRDSStateChange() {}

        @Override public void notifyCurrentPTYType(byte pty) { safePty(pty); }

        @Override public void notifyPrefabFrequency(Frequency[] arr) {}

        @Override public void notifyPrefabPTYType(byte pty) {}

        @Override public void notifyRadioPoint(RadioPoint[] arr) {}

        // Required by the vendor interface, so it cannot be removed, but
        // deliberately not forwarded. TA is decoded from the raw groups, where
        // it carries three guards (group-0 only, its own consensus, conditioned
        // on a confirmed TP). This callback would be a SECOND writer with none
        // of them, racing the decoded value, and there is no evidence it works:
        // the vendor's other getters on this unit are hollow (getRtMessage is a
        // hardcoded "", psName stays empty for a passive client, getPTYType
        // returns 0). TA lifts the user's mute, so a wrong one is expensive.
        @Override public void notifyCurrentIsTA(boolean ta) {}

        @Override public void notifyRdsShowState(boolean on) {}

        @Override public void notifyRtMessage(String rt) { safeRadioText(rt == null ? "" : rt); }

        @Override public void notifyRadioScanState(int state) { safeScanState(state); }
    };

    // ── JNI ───────────────────────────────────────────────────────────────────
    //
    // Bound with RegisterNatives from src/android/nwd.rs, NOT by exported
    // Java_... symbols. That is not a style choice: this class is loaded by a
    // fresh InMemoryDexClassLoader, and JNI's automatic symbol lookup searches
    // only the libraries loaded by the DEFINING class's loader — which for a
    // dex loaded at run time is none of them. An exported symbol would never be
    // found and every callback below would be an UnsatisfiedLinkError, in a car.
    private static native void nativeConnected(int band, int rawFreq, String ps, String rt, int pty, boolean registered);
    private static native void nativeConnectFailed(String reason);
    private static native void nativeDisconnected();
    private static native void nativeFrequency(int band, int rawFreq, String ps, int slot);
    private static native void nativeRadioText(String rt);
    private static native void nativeRdsGroup(String hex);
    private static native void nativeStereo(boolean on);
    private static native void nativePty(int pty);
    private static native void nativeScanState(int state);
    private static native void nativeRadioState(int state);
    private static native void nativeLevel(int level, int asked, int landed, boolean ok, String err);
    private static native void nativePanelKey(int key, String action);
    private static native void nativeIllumination(String action, String extras, String uiMode);
    private static native void nativeSleep(String action, String release);

    // Every crossing is wrapped. A callback runs on the VENDOR's binder thread:
    // an exception escaping into it is the vendor service's problem, not ours,
    // and it takes the radio down with it.
    private static void safeConnected(int band, int rawFreq, String ps, String rt, int pty, boolean reg) {
        try { nativeConnected(band, rawFreq, ps, rt, pty, reg); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeConnectFailed(String reason) {
        try { nativeConnectFailed(reason); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeDisconnected() {
        try { nativeDisconnected(); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeFrequency(int band, int rawFreq, String ps, int slot) {
        try { nativeFrequency(band, rawFreq, ps, slot); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeRadioText(String rt) {
        try { nativeRadioText(rt); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeRdsGroup(String hex) {
        try { nativeRdsGroup(hex); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeStereo(boolean on) {
        try { nativeStereo(on); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safePty(int pty) {
        try { nativePty(pty); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeScanState(int state) {
        try { nativeScanState(state); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeRadioState(int state) {
        try { nativeRadioState(state); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeLevel(int level, int asked, int landed, boolean ok, String err) {
        try { nativeLevel(level, asked, landed, ok, err); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safePanelKey(int key, String action) {
        try { nativePanelKey(key, action); } catch (Throwable t) { jniFailed(t); }
    }
    private static void safeSleep(String action, String release) {
        try { nativeSleep(action, release); } catch (Throwable t) { jniFailed(t); }
    }

    private static void safeIllumination(String action, String extras, String uiMode) {
        try { nativeIllumination(action, extras, uiMode); } catch (Throwable t) { jniFailed(t); }
    }

    private static void jniFailed(Throwable t) {
        // An UnsatisfiedLinkError here means RegisterNatives did not run, which
        // is a build or start-up fault rather than a device condition — log it
        // loudly once per occurrence and carry on rather than crash a car radio.
        Log.e(TAG, "native callback failed", t);
    }
}
