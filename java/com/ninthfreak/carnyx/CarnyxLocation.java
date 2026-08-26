package com.ninthfreak.carnyx;

import android.content.Context;
import android.content.pm.PackageManager;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

/**
 * Location, for the nearby picker's distances and the face's satellite glyph.
 *
 * The shape is deliberately the same as {@link NwdBridge}: static state because
 * there is one device in one process, every decision pushed into Rust through a
 * single native call, and nothing here that could be tested on a workstation
 * left on this side of the line. This file is dexed by build.rs and loaded at
 * run time through the same InMemoryDexClassLoader the tuner uses.
 *
 * <h2>Asking for the grant, and surviving not getting it</h2>
 *
 * ACCESS_COARSE_LOCATION and ACCESS_FINE_LOCATION are declared in the manifest,
 * but from API 23 declaring is not granting. An earlier version of this file
 * stopped there, reasoning that a dialog needs somebody to tap it and a head
 * unit at night on a motorway has nobody — and the result was a radio whose GPS
 * never worked at all, because the grant was never asked for even on the first
 * launch, when the driver IS sitting there looking at the screen. CarFM asks
 * (`services/gpsSession.ts`, `PermissionsAndroid.request`), and so does this.
 *
 * The ask is one-shot per launch and it never blocks. The grant arrives through
 * {@code onRequestPermissionsResult} on the Activity, which a NativeActivity app
 * does not own and cannot override, so {@link #start} polls instead: a bounded
 * number of checks on the main looper, ending the moment the grant appears or
 * the window closes. Denial is an ordinary answer — the polls stop, Rust keeps
 * reporting no fix, and the picker sorts by frequency.
 *
 * <h2>Providers</h2>
 *
 * Both GPS and NETWORK are requested when available, and whichever speaks first
 * wins. A head unit usually has a GPS antenna and often has no usable network
 * provider, but the reverse happens on units with a modem and a windscreen
 * blocking the sky, and asking for both costs one extra registration.
 */
public final class CarnyxLocation {

    private static final String TAG = "CarnyxLocation";

    /**
     * How often to accept a fix.
     *
     * A radio needs to know which town it is in, not which lane, and two seconds
     * is far coarser than a navigation app would ask for while still far finer
     * than the nearest-transmitter question needs — the FCC table's rows are
     * kilometres apart. It is also the rate at which a STOP is noticed, which is
     * why the distance filter below is zero.
     */
    private static final long MIN_INTERVAL_MS = 2000L;

    /**
     * ZERO, and it has to be.
     *
     * This was 50 metres, on the reasoning that a radio needs to know which town
     * it is in and not which lane. That is true of the POSITION and false of the
     * motion state: a parked car moves no distance at all, so it earns no further
     * callback, so the last fix — taken while still rolling — is the last word.
     * The driving glyph stayed lit after parking and reorder mode stayed refused,
     * both until the car moved another fifty metres.
     *
     * The cost of dropping the filter is a callback every {@link #MIN_INTERVAL_MS},
     * which is 0.5 Hz. CarFM's own motion service takes GPS speed at ~1 Hz and
     * calls it cheap "a car head unit's GPS is always powered".
     */
    private static final float MIN_DISTANCE_M = 0f;

    private CarnyxLocation() {}

    /**
     * How long to keep looking for the grant after asking, and how often.
     *
     * 40 × 500 ms is twenty seconds — long enough for a driver to read a dialog
     * and tap Allow, short enough that a denial stops costing anything almost at
     * once. The polls are three cheap `checkSelfPermission` calls; there is no
     * timer left running afterwards either way.
     */
    private static final int GRANT_POLLS = 40;
    private static final long GRANT_POLL_MS = 500L;

    /** Arbitrary; nothing reads it back, because the result callback is the
     *  Activity's and a NativeActivity app does not own that class. */
    private static final int REQUEST_CODE = 0x10CA;

    private static final Object LOCK = new Object();
    private static Context ctx;
    /** The Activity, kept SEPARATELY from {@link #ctx}: permissions are asked
     *  for by an Activity, and `getApplicationContext()` is not one. */
    private static android.app.Activity activity;
    private static LocationManager manager;
    private static boolean listening;
    private static boolean asked;

    /**
     * When the providers were registered, and whether anything has come back.
     *
     * <h2>Why time-to-first-fix is written down</h2>
     *
     * <p>The report was <i>"GPS still seems to take forever to indicate that it's
     * locked"</i>, and nothing in this app could say whether "forever" was twenty
     * seconds or three minutes. The log had ONE line — {@code location:
     * listening} — and then silence until a fix arrived, at which point it said
     * nothing either. A cold GNSS fix legitimately takes 30 to 90 seconds and no
     * code here makes the sky arrive sooner; what there was no excuse for is not
     * knowing which of those it was.
     *
     * <p>So the wait is measured and reported once, and while it runs the
     * satellite count is reported as it CHANGES. A unit seeing twelve satellites
     * and using none is a different fault from one seeing none at all — the first
     * is an almanac still downloading, the second is an antenna.
     */
    private static long startedAtMs;
    private static boolean firstFixNoted;

    /** The last used-in-fix count reported, so a 1 Hz callback writes one line
     *  per CHANGE rather than one per second. */
    private static int lastUsedReported = -1;

    /**
     * The GNSS status callback, as {@code Object} because its type is API 24+.
     *
     * <p>The field's TYPE is resolved when this class is loaded, and this dex is
     * built against android.jar 34 and runs on 29 — declaring it as
     * {@code GnssStatus.Callback} would be fine here, but the registration is
     * already inside a try/catch for the units that refuse it and an
     * {@code Object} keeps the whole feature in one guarded place. Null until
     * registered and after the first fix, which is when it is removed.
     */
    private static Object gnssCallback;

    /**
     * Registered from Rust; see src/android/location.rs.
     *
     * The RAW speed crosses, not a verdict about it. Deciding what counts as
     * moving needs hysteresis, hysteresis needs state, and state that decides
     * something belongs on the side of the wire that can be tested — which is
     * the rule this file's header states and the old {@code MOVING_MPS} constant
     * quietly broke.
     */
    private static native void nativePosition(
        double lat, double lon, boolean fix, float speedMps, boolean hasSpeed);

    /** One line into the diagnostics panel. The unit has no adb, so `Log.i`
     *  reaches nobody and a skipped provider looks exactly like a slow one. */
    private static native void nativeNote(String line);

    /** Every listener currently registered, so `stop` can remove all of them.
     *
     *  ONE PER PROVIDER, and not the single shared instance this used to
     *  register twice. `LocationManager` keys its registrations by the listener
     *  object, so re-registering the same one is at best ambiguous and at worst
     *  replaces the first provider with the second. A listener each costs
     *  nothing and removes the question. */
    private static final java.util.List<LocationListener> LISTENERS =
        new java.util.ArrayList<>();

    private static LocationListener newListener() {
        return new LocationListener() {
        @Override public void onLocationChanged(Location loc) {
            if (loc == null) return;
            noteFirstFix(loc);
            // Every judgement about these numbers lives in Rust
            // (`ingest_position`) — the (0, 0) a provider with nothing hands
            // back, and the speed hysteresis. Java's job is to pass on what it
            // was told.
            nativePosition(loc.getLatitude(), loc.getLongitude(), true,
                loc.hasSpeed() ? loc.getSpeed() : 0f, loc.hasSpeed());
        }

        // The three-argument overload is abstract before API 29 and default
        // after; implementing it keeps one dex working on both.
        @Override public void onStatusChanged(String provider, int status, Bundle extras) {}

        @Override public void onProviderEnabled(String provider) {}

        @Override public void onProviderDisabled(String provider) {
            // Not necessarily a loss: the other provider may still be running.
            // Ask the manager rather than assuming.
            if (!anyProviderEnabled()) {
                nativePosition(0, 0, false, 0f, false);
            }
        }
        };
    }

    /**
     * Say how long the first fix took, once, and stop counting satellites.
     *
     * <p>ONCE PER RUN, not per provider: the question is when the app could first
     * answer "where am I", and the second provider arriving later answers nothing
     * new. The provider that won is named because it matters — a NETWORK fix on a
     * unit with no sky is a different situation from a GNSS one, and the picker's
     * distances are only as good as whichever it was.
     *
     * <p>Called from the listener, which runs on the main thread, so the flags
     * need no lock of their own.
     */
    private static void noteFirstFix(Location loc) {
        if (firstFixNoted) {
            return;
        }
        firstFixNoted = true;
        long waited = startedAtMs == 0 ? -1 : android.os.SystemClock.elapsedRealtime() - startedAtMs;
        String from = loc.getProvider() == null ? "an unnamed provider" : loc.getProvider();
        try {
            nativeNote(waited < 0
                    ? "first fix from " + from
                    : "first fix from " + from + " after " + (waited / 1000) + "s");
        } catch (Throwable ignored) {
            // The note is the whole point but it is not worth a crash.
        }
        stopSatelliteWatch();
    }

    /**
     * Count satellites while there is no fix, so a long wait says WHY.
     *
     * <p>API 24+, and the unit is 29. FINE location is required — a build granted
     * COARSE only throws {@code SecurityException} here, which is an ordinary
     * outcome rather than a fault, so it is caught and noted and nothing else
     * changes. The whole feature is diagnostic: no fix depends on it and removing
     * it would cost nothing but the answer.
     *
     * <p>ONE LINE PER CHANGE IN THE USED COUNT. The callback fires about once a
     * second and the interesting number moves rarely; writing every tick would
     * push the rest of the drive out of the ring, which is the failure the log's
     * head was added to stop repeating.
     */
    private static void startSatelliteWatch(LocationManager mgr) {
        if (gnssCallback != null || android.os.Build.VERSION.SDK_INT < 24) {
            return;
        }
        try {
            android.location.GnssStatus.Callback cb = new android.location.GnssStatus.Callback() {
                @Override public void onSatelliteStatusChanged(android.location.GnssStatus status) {
                    if (firstFixNoted || status == null) {
                        return;
                    }
                    int seen = status.getSatelliteCount();
                    int used = 0;
                    for (int i = 0; i < seen; i++) {
                        if (status.usedInFix(i)) {
                            used++;
                        }
                    }
                    if (used == lastUsedReported) {
                        return;
                    }
                    lastUsedReported = used;
                    try {
                        nativeNote("acquiring — " + seen + " satellites in view, " + used + " used");
                    } catch (Throwable ignored) {
                    }
                }
            };
            mgr.registerGnssStatusCallback(cb);
            gnssCallback = cb;
        } catch (SecurityException e) {
            try { nativeNote("no satellite count: fine location not granted"); } catch (Throwable ignored) {}
        } catch (Throwable t) {
            try { nativeNote("no satellite count: " + t.getClass().getSimpleName()); } catch (Throwable ignored) {}
        }
    }

    /** Remove the satellite callback. Safe when there is none. */
    private static void stopSatelliteWatch() {
        Object cb = gnssCallback;
        gnssCallback = null;
        if (cb == null || manager == null) {
            return;
        }
        try {
            manager.unregisterGnssStatusCallback((android.location.GnssStatus.Callback) cb);
        } catch (Throwable ignored) {
            // Unregistering something already gone is not a fault.
        }
    }

    public static void attach(Context context) {
        synchronized (LOCK) {
            if (context instanceof android.app.Activity) {
                activity = (android.app.Activity) context;
            }
            if (ctx == null && context != null) {
                ctx = context.getApplicationContext();
                manager = (LocationManager) ctx.getSystemService(Context.LOCATION_SERVICE);
            }
        }
    }

    /** True when the manifest permissions have actually been granted. */
    public static boolean hasPermission() {
        if (ctx == null) return false;
        return ctx.checkSelfPermission("android.permission.ACCESS_FINE_LOCATION")
                    == PackageManager.PERMISSION_GRANTED
            || ctx.checkSelfPermission("android.permission.ACCESS_COARSE_LOCATION")
                    == PackageManager.PERMISSION_GRANTED;
    }

    private static boolean anyProviderEnabled() {
        LocationManager m;
        synchronized (LOCK) { m = manager; }
        if (m == null) return false;
        try {
            return m.isProviderEnabled(LocationManager.GPS_PROVIDER)
                || m.isProviderEnabled(LocationManager.NETWORK_PROVIDER);
        } catch (Throwable t) {
            return false;
        }
    }

    /**
     * Ask for the grant, once per launch, and watch for it to land.
     *
     * Returns false when there is nothing to ask with (no Activity) or the ask
     * has already been made. Never blocks: {@code requestPermissions} puts a
     * dialog up and returns immediately, and the polling below is what notices
     * the answer.
     */
    private static boolean requestGrant() {
        final android.app.Activity act;
        synchronized (LOCK) {
            if (asked || activity == null) return false;
            asked = true;
            act = activity;
        }
        final Handler h = new Handler(Looper.getMainLooper());
        h.post(new Runnable() {
            @Override public void run() {
                try {
                    act.requestPermissions(new String[] {
                        "android.permission.ACCESS_FINE_LOCATION",
                        "android.permission.ACCESS_COARSE_LOCATION",
                    }, REQUEST_CODE);
                } catch (Throwable t) {
                    Log.w(TAG, "requestPermissions failed", t);
                    return;
                }
                h.postDelayed(new Runnable() {
                    int left = GRANT_POLLS;
                    @Override public void run() {
                        if (hasPermission()) {
                            // The ask succeeded; registration is start()'s job
                            // and it is now free to do it.
                            start();
                            return;
                        }
                        if (--left > 0) h.postDelayed(this, GRANT_POLL_MS);
                        else Log.i(TAG, "location was not granted");
                    }
                }, GRANT_POLL_MS);
            }
        });
        return true;
    }

    /**
     * Begin listening. Returns false when there is nothing to listen to, which
     * is a normal state and not an error: no provider, no LocationManager, or a
     * grant that has been asked for and not yet given.
     *
     * Idempotent. Registration must happen on a thread with a Looper, and the
     * caller is a JNI thread that has none, so it is posted to the main thread.
     */
    public static boolean start() {
        LocationManager m;
        synchronized (LOCK) {
            if (listening) return true;
            m = manager;
        }
        if (m == null) return false;
        if (!hasPermission()) {
            // Not a failure yet — ask, and let the poll above call back in.
            requestGrant();
            return false;
        }

        final LocationManager mgr = m;
        new Handler(Looper.getMainLooper()).post(new Runnable() {
            @Override public void run() {
                StringBuilder live = new StringBuilder();
                boolean any = false;
                for (String p : new String[] {
                        LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER }) {
                    try {
                        // REGISTERED WHETHER OR NOT IT IS ENABLED RIGHT NOW, and
                        // this line is the whole bug it replaces.
                        //
                        // It used to read `if (!mgr.isProviderEnabled(p))
                        // continue;`. On a head unit that has just powered up,
                        // location services come up asynchronously and GPS_PROVIDER
                        // can still report disabled when the app starts — so GPS
                        // was skipped, NETWORK was registered, `listening` went
                        // true, and NOTHING EVER TRIED AGAIN: `start` is called
                        // exactly once from `android_main`, it returns early while
                        // `listening`, and `onProviderEnabled` only reaches a
                        // listener that is already registered. The result is a
                        // whole drive on the network provider, which on a car
                        // without connectivity is no fix at all.
                        //
                        // Registering a disabled provider is legal and is what the
                        // reference does (`VibeStreamModule.startGps` calls
                        // `requestLocationUpdates(GPS_PROVIDER, ...)` with no such
                        // check). Updates simply do not arrive until it comes up —
                        // and because we are registered, `onProviderEnabled` does.
                        LocationListener l = newListener();
                        mgr.requestLocationUpdates(p, MIN_INTERVAL_MS, MIN_DISTANCE_M, l);
                        synchronized (LOCK) { LISTENERS.add(l); }
                        any = true;
                        if (live.length() > 0) live.append(", ");
                        live.append(p).append(mgr.isProviderEnabled(p) ? "" : " (off for now)");
                        // A last-known fix is worth more than nothing while the
                        // first real one is still being acquired: a cold GPS can
                        // take a minute, and the picker is unusable until then.
                        //
                        // AND IT IS SAID OUT LOUD, because it is not the same
                        // thing as a fix. A last-known position can be hours old
                        // and a hundred miles away — the unit was parked at an
                        // airport — and the face's glyph lights for it just the
                        // same. A driver reading "locked" needs to be able to tell
                        // this from the real one, and `first fix from …` below is
                        // the line that arrives when the sky answers.
                        Location last = mgr.getLastKnownLocation(p);
                        if (last != null) {
                            nativePosition(last.getLatitude(), last.getLongitude(), true,
                                last.hasSpeed() ? last.getSpeed() : 0f, last.hasSpeed());
                            try {
                                long age = android.os.SystemClock.elapsedRealtime()
                                        - last.getElapsedRealtimeNanos() / 1_000_000L;
                                nativeNote("seeded from " + p + "'s last known fix, "
                                        + (age / 1000) + "s old — not a lock");
                            } catch (Throwable ignored) {
                            }
                        }
                    } catch (SecurityException e) {
                        // Permission revoked between the check and here.
                        Log.w(TAG, "location denied for " + p, e);
                    } catch (Throwable t) {
                        // An IllegalArgumentException here means the unit has no
                        // such provider, which is ordinary. Keep going.
                        Log.w(TAG, "requestLocationUpdates failed for " + p, t);
                    }
                }
                synchronized (LOCK) { listening = any; }
                if (any) {
                    // THE CLOCK STARTS HERE, not at `start()`: everything above
                    // is posted to the main thread and the wait being measured is
                    // the sky's, not the queue's. `elapsedRealtime` rather than
                    // wall time, so a clock the MCU corrects mid-acquisition
                    // cannot produce a negative answer.
                    startedAtMs = android.os.SystemClock.elapsedRealtime();
                    startSatelliteWatch(mgr);
                }
                try {
                    nativeNote(any ? "listening on " + live : "no provider would register");
                } catch (Throwable ignored) {}
                if (!any) {
                    nativePosition(0, 0, false, 0f, false);
                    // AND TRY AGAIN, because `start` is called exactly once from
                    // `android_main` and there is no other path back here. A unit
                    // whose location service is not up yet used to lose GPS for the
                    // whole session on the strength of one early answer.
                    retry();
                }
            }
        });
        return true;
    }

    /**
     * How long to keep trying when no provider would register at all, and how
     * often. 12 x 10s is two minutes — past any plausible boot, and bounded so a
     * unit with location switched off is not polled forever.
     */
    private static final int START_RETRIES = 12;
    private static final long START_RETRY_MS = 10_000L;

    private static int retriesLeft = START_RETRIES;

    /** Re-attempt registration later. Bounded, and stops the moment it works. */
    private static void retry() {
        synchronized (LOCK) {
            if (retriesLeft <= 0) return;
            retriesLeft--;
        }
        new Handler(Looper.getMainLooper()).postDelayed(new Runnable() {
            @Override public void run() {
                synchronized (LOCK) { if (listening) return; }
                start();
            }
        }, START_RETRY_MS);
    }

    public static void stop() {
        LocationManager m;
        synchronized (LOCK) {
            if (!listening) return;
            m = manager;
            listening = false;
        }
        if (m == null) return;
        java.util.List<LocationListener> gone;
        synchronized (LOCK) {
            gone = new java.util.ArrayList<>(LISTENERS);
            LISTENERS.clear();
        }
        for (LocationListener l : gone) {
            try {
                m.removeUpdates(l);
            } catch (Throwable t) {
                Log.w(TAG, "removeUpdates failed", t);
            }
        }
        // The satellite watch goes with them. It normally removes itself at the
        // first fix; this is the path where there never was one.
        stopSatelliteWatch();
    }
}
