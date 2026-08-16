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
 * <h2>Why this asks for permission and never demands it</h2>
 *
 * ACCESS_COARSE_LOCATION and ACCESS_FINE_LOCATION are declared in the manifest,
 * but from API 23 declaring is not granting — the grant needs a dialog, and a
 * dialog needs somebody to tap it. On a head unit, at night, on a motorway,
 * that somebody does not exist. So every entry point here treats "not granted"
 * as an ordinary answer: {@link #start} returns false, Rust keeps the fake it
 * already had, and the picker keeps sorting by frequency. Nothing throws and
 * nothing blocks.
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
     * How often to accept a fix, and how far the car must move to earn one.
     *
     * A radio needs to know which town it is in, not which lane. Two seconds and
     * fifty metres is far coarser than a navigation app would ask for and is
     * still far finer than the nearest-transmitter question needs — the FCC
     * table's rows are kilometres apart. The cost of asking for more is battery
     * on a device that is often running off the ignition.
     */
    private static final long MIN_INTERVAL_MS = 2000L;
    private static final float MIN_DISTANCE_M = 50f;

    /**
     * Above this, the car is moving, and the face shows the driving glyph.
     *
     * 2.2 m/s is about 8 km/h — above walking pace, below anything that could be
     * a stationary unit's GPS noise. Speed is only reported by some providers,
     * so a fix without it is treated as stationary rather than guessed at.
     */
    private static final float MOVING_MPS = 2.2f;

    private CarnyxLocation() {}

    private static final Object LOCK = new Object();
    private static Context ctx;
    private static LocationManager manager;
    private static boolean listening;

    /** Registered from Rust; see src/android/location.rs. */
    private static native void nativePosition(double lat, double lon, boolean fix, boolean inMotion);

    private static final LocationListener LISTENER = new LocationListener() {
        @Override public void onLocationChanged(Location loc) {
            if (loc == null) return;
            boolean moving = loc.hasSpeed() && loc.getSpeed() >= MOVING_MPS;
            // Every sanity check on the numbers themselves lives in Rust
            // (`ingest_position`), including the (0, 0) that a provider with
            // nothing hands back. Java's job is to pass on what it was told.
            nativePosition(loc.getLatitude(), loc.getLongitude(), true, moving);
        }

        // The three-argument overload is abstract before API 29 and default
        // after; implementing it keeps one dex working on both.
        @Override public void onStatusChanged(String provider, int status, Bundle extras) {}

        @Override public void onProviderEnabled(String provider) {}

        @Override public void onProviderDisabled(String provider) {
            // Not necessarily a loss: the other provider may still be running.
            // Ask the manager rather than assuming.
            if (!anyProviderEnabled()) {
                nativePosition(0, 0, false, false);
            }
        }
    };

    public static void attach(Context context) {
        synchronized (LOCK) {
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
     * Begin listening. Returns false when there is nothing to listen to, which
     * is a normal state and not an error: no permission, no provider, or no
     * LocationManager at all.
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
        if (m == null || !hasPermission()) return false;

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
                            nativePosition(last.getLatitude(), last.getLongitude(), true, false);
                        }
                    } catch (SecurityException e) {
                        // Permission revoked between the check and here.
                        Log.w(TAG, "location denied for " + p, e);
                    } catch (Throwable t) {
                        Log.w(TAG, "requestLocationUpdates failed for " + p, t);
                    }
                }
                synchronized (LOCK) { listening = any; }
                if (!any) nativePosition(0, 0, false, false);
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
