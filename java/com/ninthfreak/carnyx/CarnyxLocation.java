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

    private static final LocationListener LISTENER = new LocationListener() {
        @Override public void onLocationChanged(Location loc) {
            if (loc == null) return;
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
                boolean any = false;
                for (String p : new String[] {
                        LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER }) {
                    try {
                        if (!mgr.isProviderEnabled(p)) continue;
                        mgr.requestLocationUpdates(p, MIN_INTERVAL_MS, MIN_DISTANCE_M, LISTENER);
                        any = true;
                        // A last-known fix is worth more than nothing while the
                        // first real one is still being acquired: a cold GPS can
                        // take a minute, and the picker is unusable until then.
                        Location last = mgr.getLastKnownLocation(p);
                        if (last != null) {
                            nativePosition(last.getLatitude(), last.getLongitude(), true,
                                last.hasSpeed() ? last.getSpeed() : 0f, last.hasSpeed());
                        }
                    } catch (SecurityException e) {
                        // Permission revoked between the check and here.
                        Log.w(TAG, "location denied for " + p, e);
                    } catch (Throwable t) {
                        Log.w(TAG, "requestLocationUpdates failed for " + p, t);
                    }
                }
                synchronized (LOCK) { listening = any; }
                if (!any) nativePosition(0, 0, false, 0f, false);
            }
        });
        return true;
    }

    public static void stop() {
        LocationManager m;
        synchronized (LOCK) {
            if (!listening) return;
            m = manager;
            listening = false;
        }
        if (m == null) return;
        try {
            m.removeUpdates(LISTENER);
        } catch (Throwable t) {
            Log.w(TAG, "removeUpdates failed", t);
        }
    }
}
