package com.ninthfreak.carnyx;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ProviderInfo;
import android.database.ContentObserver;
import android.database.Cursor;
import android.net.Uri;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.Looper;
import android.util.Log;

import java.lang.ref.WeakReference;

/**
 * Turn-by-turn navigation, from CoMaps, over its content provider (#134).
 *
 * <h2>What this class is and is not</h2>
 *
 * <p>It is a WIRE, on {@link CarnyxNav}'s rule. It watches one URI, reads one
 * row when CoMaps says it changed, and hands every column across as a name and
 * a string. It decides nothing: not what {@code TURN_RIGHT} draws, not what
 * "0.2 mi" is in metres, not whether {@code OnRoute} means a route. All of that
 * is {@code src/comaps.rs}, where it is tested on a machine with no CoMaps.
 *
 * <h2>The provider, read out of CoMaps' own source</h2>
 *
 * <pre>
 *   &lt;permission android:name="${applicationId}.permission.READ_NAVIGATION_DATA"
 *       android:protectionLevel="dangerous" /&gt;
 *   &lt;provider android:name=".provider.NavigationContentProvider"
 *       android:authorities="${applicationId}.provider.navigation"
 *       android:exported="true"
 *       android:readPermission="${applicationId}.permission.READ_NAVIGATION_DATA" /&gt;
 * </pre>
 *
 * <p>First shipped in CoMaps v2026.07.16. One row at {@code /live}; the column
 * set grew in v2026.08.31, which is why the row crosses by NAME.
 *
 * <h2>WATCHED, NEVER POLLED — and the reason is a CoMaps bug</h2>
 *
 * <p>CoMaps' {@code NavigationService.onLocationUpdated} refreshes its cached
 * route info and calls {@code notifyChange} on {@code /live} on every GPS fix
 * while navigating. That is the whole of this class's liveness: a change
 * arrives, the row is read, and when CoMaps stops navigating the changes stop
 * and {@code Nav::EXPIRY} clears the face.
 *
 * <p>POLLING WOULD BE WRONG, not merely wasteful. {@code RoutingController}
 * never clears {@code mCachedRoutingInfo}, so after a route ends the provider
 * goes on answering with the last row — {@code session_state} still
 * {@code OnRoute} — for as long as CoMaps runs. A 1 Hz poll like OsmAnd's
 * would draw a finished route forever. Nothing is queried here except in
 * answer to a change.
 *
 * <p>IT ALSO COSTS NOTHING WHILE IDLE. Registering an observer is a call into
 * {@code system_server}; it does not start CoMaps' process, and nothing is
 * read until CoMaps itself — navigating, so already running — says to.
 *
 * <h2>The permission</h2>
 *
 * <p>DANGEROUS, SO A DIALOG. {@code registerContentObserver} on another app's
 * authority is checked against that provider's permissions for any caller
 * targeting API 26 or later, and throws without it — so the observer is the
 * test of the grant as well as the feed. Until the grant lands the worker
 * re-checks every {@link #PERMISSION_RECHECK_MS}: there is no result callback
 * to wait on, because {@code onRequestPermissionsResult} lands on Slint's
 * {@code NativeActivity} and not on anything of ours ({@link CarnyxAlert}'s
 * note on the same problem).
 *
 * <p>THE ASK IS ONLY EVER A TAP, never automatic: {@link #requestPermission}
 * is called when the driver chooses CoMaps or taps its row.
 *
 * <h2>Four package names, tried in order</h2>
 *
 * <p>CoMaps' {@code android/app/build.gradle.kts} builds one app under four
 * ids: {@code app.comaps} plus {@code .fdroid}, {@code .google} or
 * {@code .huawei}, and none for the "web" APK. Same code, separate apps. The
 * authority and the permission both carry the id, so each is declared in both
 * manifests and the first one installed WITH the provider is used.
 *
 * <h2>What is confirmed, and what is not</h2>
 *
 * <p>NOTHING HERE HAS RUN AGAINST COMAPS. The provider, its columns, its
 * permission and the push-per-fix were read in CoMaps' source; that is a
 * reading, not a measurement. Unsettled until a drive: whether Android 10
 * offers the dialog for a permission whose app was installed AFTER Carnyx (if
 * not, reinstalling Carnyx fixes it), and how often the fixes actually come.
 */
public final class CarnyxComaps {
    private static final String TAG = "CarnyxComaps";

    /** Every CoMaps application id, most-likely-on-a-head-unit first. */
    private static final String[] COMAPS_PACKAGES = {
        "app.comaps.fdroid",
        "app.comaps.google",
        "app.comaps",
        "app.comaps.huawei",
    };

    private static final String AUTHORITY_SUFFIX = ".provider.navigation";
    private static final String PERMISSION_SUFFIX = ".permission.READ_NAVIGATION_DATA";

    /** The link states {@code crate::comaps::Link::from_code} reads. */
    static final int LINK_OFF = 0;
    static final int LINK_NEEDS_PERMISSION = 1;
    static final int LINK_WATCHING = 2;
    static final int LINK_FAILED = 3;

    /**
     * How often the worker re-checks a missing grant.
     *
     * <p>TWO SECONDS: short enough that a driver who taps Allow sees the mark
     * light before they have looked away, and a {@code checkSelfPermission}
     * every two seconds is nothing — it is a cached lookup, not a round trip
     * into CoMaps.
     */
    private static final long PERMISSION_RECHECK_MS = 2000L;

    /** How long to wait before retrying a registration that failed for some other reason. */
    private static final long RETRY_MS = 30000L;

    /** Ours alone; {@link CarnyxLocation} uses 0x10CA and {@link CarnyxAlert} 1. */
    private static final int REQUEST_CODE = 0x0C0A;

    private static Context ctx;
    private static WeakReference<Activity> activity;

    /** The worker: registration, re-checks and every query run here, off the UI thread. */
    private static HandlerThread thread;
    private static volatile Handler handler;

    /**
     * Restart guard, {@link CarnyxNav#pollGen}'s: every start and stop bumps it,
     * and a task or an observer from an older generation does nothing. Without
     * it a switch-off-and-on mid-registration would leave two observers.
     */
    private static volatile int gen;

    /** The package being watched (or waited on), or empty when stopped. */
    private static String watching = "";
    private static ContentObserver observer;
    private static int link = LINK_OFF;

    /** Edge-triggered, like {@link CarnyxNav}'s: one line per fault, one per recovery. */
    private static volatile boolean queryFaultReported;
    private static volatile boolean failReported;

    private CarnyxComaps() {
    }

    /**
     * One {@code /live} row: column names, and each cell as a string or null.
     * An EMPTY pair is an empty cursor — CoMaps has no route cached.
     */
    private static native void nativeComapsRow(String[] names, String[] values);

    /** The link state, as one of the {@code LINK_*} codes. Sent on a change only. */
    private static native void nativeComapsLink(int state);

    /** One line into the diagnostics panel; logcat reaches nobody here. */
    private static native void nativeComapsNote(String line);

    /** Hand the class the app context and, when it is one, the Activity. */
    public static synchronized void attach(Context context) {
        if (context == null) {
            return;
        }
        if (ctx == null) {
            ctx = context.getApplicationContext();
        }
        if (context instanceof Activity) {
            activity = new WeakReference<>((Activity) context);
        }
    }

    /**
     * Which CoMaps has the navigation provider, or {@code ""}.
     *
     * <p>THE PROVIDER IS THE TEST, not the package. A CoMaps from before
     * v2026.07.16 is installed and has nothing to read; {@link #outdatedPackage}
     * names that case separately, because "update CoMaps" and "install CoMaps"
     * are different instructions.
     */
    public static synchronized String installedPackage() {
        if (ctx == null) {
            return "";
        }
        PackageManager pm = ctx.getPackageManager();
        for (String pkg : COMAPS_PACKAGES) {
            try {
                ProviderInfo info = pm.resolveContentProvider(pkg + AUTHORITY_SUFFIX, 0);
                if (info != null && pkg.equals(info.packageName)) {
                    return pkg;
                }
            } catch (Throwable t) {
                Log.w(TAG, "resolveContentProvider failed for " + pkg, t);
            }
        }
        return "";
    }

    /** A CoMaps installed WITHOUT the provider, or {@code ""}. See {@link #installedPackage}. */
    public static synchronized String outdatedPackage() {
        if (ctx == null) {
            return "";
        }
        PackageManager pm = ctx.getPackageManager();
        for (String pkg : COMAPS_PACKAGES) {
            try {
                pm.getPackageInfo(pkg, 0);
            } catch (PackageManager.NameNotFoundException absent) {
                continue;
            } catch (Throwable t) {
                continue;
            }
            try {
                if (pm.resolveContentProvider(pkg + AUTHORITY_SUFFIX, 0) == null) {
                    return pkg;
                }
            } catch (Throwable ignored) {
                return pkg;
            }
        }
        return "";
    }

    /**
     * Start watching. Idempotent.
     *
     * @return a line for the diagnostics log, never null. The registration is
     *     on the worker and reports itself through {@link #nativeComapsLink}.
     */
    public static synchronized String start() {
        if (ctx == null) {
            return "no context — attach() has not run";
        }
        if (handler != null) {
            return "already watching for " + watching;
        }
        String pkg = installedPackage();
        if (pkg.isEmpty()) {
            String old = outdatedPackage();
            setLinkLocked(LINK_OFF);
            // NOT AN ERROR, worded so it does not read as one — the same rule
            // `CarnyxNav.start` states for a missing OsmAnd.
            return old.isEmpty()
                ? "CoMaps is not installed — nothing to watch"
                : old + " is too old to share its navigation (it needs v2026.07.16 or newer)";
        }
        final int myGen = ++gen;
        watching = pkg;
        queryFaultReported = false;
        failReported = false;
        thread = new HandlerThread("carnyx-comaps");
        thread.start();
        handler = new Handler(thread.getLooper());
        handler.post(new Runnable() {
            @Override
            public void run() {
                attempt(myGen);
            }
        });
        return "watching for " + pkg;
    }

    /** Stop watching and let the worker go. Safe when nothing was started. */
    public static synchronized String stop() {
        gen++;
        String was = watching;
        unregisterLocked();
        Handler h = handler;
        handler = null;
        if (h != null) {
            h.removeCallbacksAndMessages(null);
        }
        HandlerThread t = thread;
        thread = null;
        if (t != null) {
            t.quit();
        }
        watching = "";
        setLinkLocked(LINK_OFF);
        return was.isEmpty() ? "not watching" : "stopped watching " + was;
    }

    /**
     * Ask the driver for the permission. The CoMaps row's tap.
     *
     * <p>ON THE MAIN LOOPER, because {@code requestPermissions} starts an
     * activity for its result and this is called from Rust's event loop, which
     * is not Android's UI thread — {@link CarnyxLocation#requestGrant} posts
     * for the same reason.
     *
     * <p>ANSWERS THAT ARE NOT A DIALOG come back as a line, {@link CarnyxAlert}'s
     * rule: already granted, nothing installed, no Activity to ask with.
     */
    public static synchronized String requestPermission() {
        if (ctx == null) {
            return "no context — attach() has not run";
        }
        String pkg = installedPackage();
        if (pkg.isEmpty()) {
            String old = outdatedPackage();
            return old.isEmpty()
                ? "CoMaps is not installed — no permission to ask for"
                : old + " is too old to share its navigation — no permission to ask for";
        }
        final String perm = pkg + PERMISSION_SUFFIX;
        try {
            if (ctx.checkSelfPermission(perm) == PackageManager.PERMISSION_GRANTED) {
                return "permission already granted (" + perm + ")";
            }
        } catch (Throwable t) {
            return "could not check " + perm + " — " + why(t);
        }
        final Activity a = activity == null ? null : activity.get();
        if (a == null) {
            return "permission NOT granted, and no Activity to ask with — grant it"
                    + " in Android's own app settings for Carnyx";
        }
        new Handler(Looper.getMainLooper()).post(new Runnable() {
            @Override
            public void run() {
                try {
                    a.requestPermissions(new String[] { perm }, REQUEST_CODE);
                } catch (Throwable t) {
                    safeNote("CoMaps: permission request failed — " + why(t));
                }
            }
        });
        return "asked for " + perm + ". Tap Allow if a dialog appeared; Android stops"
                + " showing it after two refusals, and then it has to be granted in"
                + " Android's own app settings for Carnyx";
    }

    /**
     * Bring CoMaps to the front — the status-bar mark's tap. Works whether or
     * not the feed is watched, on {@link CarnyxNav#launch}'s reasoning.
     */
    public static synchronized String launch() {
        if (ctx == null) {
            return "no context — attach() has not run";
        }
        String pkg = watching.isEmpty() ? installedPackage() : watching;
        if (pkg.isEmpty()) {
            pkg = outdatedPackage();
        }
        if (pkg.isEmpty()) {
            return "CoMaps is not installed — nothing to launch";
        }
        Intent launch = ctx.getPackageManager().getLaunchIntentForPackage(pkg);
        if (launch == null) {
            return pkg + " has no launcher activity";
        }
        launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        try {
            ctx.startActivity(launch);
            return "launched " + pkg;
        } catch (Throwable t) {
            return "startActivity failed: " + why(t);
        }
    }

    /**
     * Register the observer, or wait for the grant. On the worker.
     *
     * <p>THE GRANT IS CHECKED FIRST, though registering would throw anyway,
     * so a missing permission is the ordinary waiting state and not a fault
     * line every two seconds.
     */
    private static void attempt(final int myGen) {
        final String pkg;
        final Handler h;
        synchronized (CarnyxComaps.class) {
            if (myGen != gen || handler == null) {
                return;
            }
            pkg = watching;
            h = handler;
        }
        Runnable again = new Runnable() {
            @Override
            public void run() {
                attempt(myGen);
            }
        };
        boolean granted;
        try {
            granted = ctx.checkSelfPermission(pkg + PERMISSION_SUFFIX)
                    == PackageManager.PERMISSION_GRANTED;
        } catch (Throwable t) {
            granted = false;
        }
        if (!granted) {
            setLink(myGen, LINK_NEEDS_PERMISSION);
            h.postDelayed(again, PERMISSION_RECHECK_MS);
            return;
        }
        final Uri uri = liveUri(pkg);
        ContentObserver o = new ContentObserver(h) {
            @Override
            public void onChange(boolean selfChange) {
                queryOnce(myGen, uri);
            }
        };
        try {
            ctx.getContentResolver().registerContentObserver(uri, false, o);
        } catch (Throwable t) {
            setLink(myGen, LINK_FAILED);
            if (!failReported) {
                failReported = true;
                safeNote("CoMaps: could not watch " + uri + " — " + why(t)
                        + "; retrying every " + (RETRY_MS / 1000L) + " s");
            }
            h.postDelayed(again, RETRY_MS);
            return;
        }
        synchronized (CarnyxComaps.class) {
            if (myGen != gen) {
                // Stopped while registering: hand it straight back.
                try {
                    ctx.getContentResolver().unregisterContentObserver(o);
                } catch (Throwable ignored) {
                }
                return;
            }
            unregisterLocked();
            observer = o;
        }
        failReported = false;
        setLink(myGen, LINK_WATCHING);
        safeNote("CoMaps: watching " + uri);
    }

    /**
     * Read the row and hand it across. On the worker, in answer to a change.
     *
     * <p>EVERY CELL AS {@code getString}, null kept as null. The cursor arrives
     * through a {@code CursorWindow}, which answers {@code getString} for its
     * integer and float cells with their decimal text, so nothing about a
     * column's type has to be known on this side.
     */
    private static void queryOnce(int myGen, Uri uri) {
        if (myGen != gen) {
            return;
        }
        Cursor c = null;
        try {
            c = ctx.getContentResolver().query(uri, null, null, null, null);
            if (c == null) {
                if (!queryFaultReported) {
                    queryFaultReported = true;
                    safeNote("CoMaps: " + uri + " answered no cursor");
                }
                return;
            }
            String[] names = c.getColumnNames();
            if (names == null || !c.moveToFirst()) {
                safeRow(new String[0], new String[0]);
            } else {
                String[] values = new String[names.length];
                for (int i = 0; i < names.length; i++) {
                    values[i] = c.isNull(i) ? null : c.getString(i);
                }
                safeRow(names, values);
            }
            if (queryFaultReported) {
                queryFaultReported = false;
                safeNote("CoMaps: feed recovered");
            }
        } catch (SecurityException revoked) {
            // THE GRANT WAS TAKEN BACK while watching — Android's app settings.
            // Back to waiting for it, rather than a fault line a second.
            synchronized (CarnyxComaps.class) {
                if (myGen != gen) {
                    return;
                }
                unregisterLocked();
            }
            setLink(myGen, LINK_NEEDS_PERMISSION);
            safeNote("CoMaps: permission withdrawn — " + why(revoked));
            final int g = myGen;
            Handler h = handler;
            if (h != null) {
                h.postDelayed(new Runnable() {
                    @Override
                    public void run() {
                        attempt(g);
                    }
                }, PERMISSION_RECHECK_MS);
            }
        } catch (Throwable t) {
            if (!queryFaultReported) {
                queryFaultReported = true;
                safeNote("CoMaps: reading " + uri + " failed — " + why(t));
            }
        } finally {
            if (c != null) {
                try {
                    c.close();
                } catch (Throwable ignored) {
                }
            }
        }
    }

    private static Uri liveUri(String pkg) {
        return Uri.parse("content://" + pkg + AUTHORITY_SUFFIX + "/live");
    }

    /** Let the observer go. Called with the class lock held. */
    private static void unregisterLocked() {
        ContentObserver o = observer;
        observer = null;
        if (o != null && ctx != null) {
            try {
                ctx.getContentResolver().unregisterContentObserver(o);
            } catch (Throwable ignored) {
                // Not registered is not a fault.
            }
        }
    }

    /** A link edge from the worker, dropped if a stop or restart has superseded it. */
    private static void setLink(int myGen, int state) {
        synchronized (CarnyxComaps.class) {
            if (myGen != gen) {
                return;
            }
            setLinkLocked(state);
        }
    }

    /** Send the state on a CHANGE only. Called with the class lock held. */
    private static void setLinkLocked(int state) {
        if (link == state) {
            return;
        }
        link = state;
        try {
            nativeComapsLink(state);
        } catch (Throwable t) {
            Log.w(TAG, "link state not delivered", t);
        }
    }

    private static void safeRow(String[] names, String[] values) {
        try {
            nativeComapsRow(names, values);
        } catch (Throwable t) {
            Log.w(TAG, "row not delivered", t);
        }
    }

    /** A note that cannot itself break the caller. See {@code NwdBridge.safe*}. */
    private static void safeNote(String line) {
        try {
            nativeComapsNote(line);
        } catch (Throwable t) {
            Log.w(TAG, "note failed: " + line, t);
        }
    }

    /** A throwable as one short line. See {@code NwdBridge.why}. */
    private static String why(Throwable t) {
        if (t == null) {
            return "unknown";
        }
        StringBuilder b = new StringBuilder(t.getClass().getSimpleName());
        if (t.getMessage() != null && !t.getMessage().isEmpty()) {
            b.append(": ").append(t.getMessage());
        }
        return b.length() > 120 ? b.substring(0, 120) + "…" : b.toString();
    }
}
