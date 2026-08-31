package com.ninthfreak.carnyx;

import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.ServiceConnection;
import android.content.pm.PackageManager;
import android.os.Bundle;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.IBinder;
import android.os.RemoteException;
import android.os.SystemClock;
import android.util.Log;

import net.osmand.aidlapi.IOsmAndAidlCallback;
import net.osmand.aidlapi.IOsmAndAidlInterface;
import net.osmand.aidlapi.gpx.AGpxBitmap;
import net.osmand.aidlapi.info.AppInfoParams;
import net.osmand.aidlapi.logcat.OnLogcatMessageParams;
import net.osmand.aidlapi.navigation.ADirectionInfo;
import net.osmand.aidlapi.navigation.ANavigationUpdateParams;
import net.osmand.aidlapi.navigation.ANavigationVoiceRouterMessageParams;
import net.osmand.aidlapi.navigation.OnVoiceNavigationParams;
import net.osmand.aidlapi.search.SearchResult;

import java.util.List;

/**
 * Turn-by-turn navigation, from OsmAnd, over its AIDL API.
 *
 * <h2>What this class is and is not</h2>
 *
 * <p>It is a WIRE. It binds OsmAnd's service, subscribes to two callbacks, and
 * hands what arrives to Rust unaltered — three integers and two lists of
 * strings. It decides nothing: not what turn type 13 means, not whether a
 * {@code -1} is a turn, not which of the voice router's two lists is the
 * instruction, not when a route has gone stale. All of that is {@code src/nav.rs},
 * where it is tested on a machine with no head unit and no OsmAnd.
 *
 * <p>That is the same rule {@link CarnyxLocation} states for the motion verdict,
 * and the reason is the same: a decision made on this side cannot be tested at
 * all.
 *
 * <h2>Which OsmAnd, and which service</h2>
 *
 * <p>THE V2 SERVICE, which is the one that speaks {@code net.osmand.aidlapi}.
 * OsmAnd's own manifest declares both:
 *
 * <pre>
 *   &lt;service android:name="net.osmand.aidl.OsmandAidlService"   android:exported="true"&gt;
 *   &lt;service android:name="net.osmand.aidl.OsmandAidlServiceV2" android:exported="true"&gt;
 * </pre>
 *
 * <p>and {@code OsmandAidlServiceV2.java} imports
 * {@code net.osmand.aidlapi.IOsmAndAidlInterface} where the V1 service imports
 * {@code net.osmand.aidl.*}. Binding V1 with a V2 interface would hand back a
 * binder whose descriptor does not match and every call would throw.
 *
 * <p>THE BIND IS OPEN AND EVERY CALL IS GATED, and an earlier version of this
 * comment had that half wrong — it said "no whitelist, no plugin to enable",
 * read off {@code onBind} alone, and the first drive with a route disproved it.
 * {@code OsmandAidlServiceV2.getApi} checks {@code isAppEnabled(callingPackage)}
 * on EVERY method: an unknown caller is added to OsmAnd's connected-apps list
 * DISABLED ({@code new ConnectedApp(app, pack, false)}), saved, and refused —
 * registrations return -1 and {@code getAppInfo} returns null, silently, until
 * the driver opens OsmAnd's Plugins screen and switches this app on. The
 * refused state is detected below, reported on the settings row, and retried
 * once the poll starts answering. {@code tools/check-osmand-aidl.sh} pins the
 * gate against upstream.
 *
 * <h2>Four package names, tried in order</h2>
 *
 * <p>OsmAnd ships under several application ids from one build
 * ({@code OsmAnd/build.gradle}'s {@code productFlavors}) and a driver may have
 * any of them. They are tried in the order below, which is "most likely on a
 * head unit" first: the paid build, then the free one, then the Huawei edition,
 * then a nightly. WHICH ONE ANSWERED IS REPORTED, because "OsmAnd is not
 * installed" and "the wrong OsmAnd is installed" need different answers from a
 * driver and look identical otherwise.
 *
 * <h2>Package visibility</h2>
 *
 * <p>From targetSdk 30 a package cannot see another it has not declared, and the
 * filtering is SILENT — {@code bindService} simply returns false. All four ids
 * are declared in {@code <queries>} in both manifests for exactly that reason,
 * the same way the vendor radio service is.
 *
 * <h2>What is confirmed, and what is not</h2>
 *
 * <p>NOTHING HERE HAS RUN AGAINST OSMAND. The interface, the callback, the
 * parcelables and the service declaration were read out of the upstream
 * repository and are checked against it by {@code tools/check-osmand-aidl.sh};
 * that is a reading, not a measurement. The first drive is what settles whether
 * the binder answers.
 */
public final class CarnyxNav {
    private static final String TAG = "CarnyxNav";

    /**
     * The V2 service's action, which is also its class name. See the class note.
     */
    private static final String OSMAND_SERVICE_V2 = "net.osmand.aidl.OsmandAidlServiceV2";

    /**
     * Every application id OsmAnd ships under, most-likely first.
     *
     * <p>From {@code OsmAnd/build.gradle}: {@code net.osmand.plus} is both
     * "OsmAnd+" (paid) and "OsmAnd~" (F-Droid/nightly-free), {@code net.osmand}
     * is the free Play build, {@code net.osmand.huawei} is the AppGallery
     * edition and {@code net.osmand.dev} is the nightly.
     */
    private static final String[] OSMAND_PACKAGES = {
        "net.osmand.plus",
        "net.osmand",
        "net.osmand.huawei",
        "net.osmand.dev",
    };

    private static Context ctx;
    private static IOsmAndAidlInterface osmand;
    private static boolean binding;
    private static String boundPackage = "";

    /**
     * The ids OsmAnd hands back when we subscribe, needed to unsubscribe.
     *
     * <p>Kept so {@link #stop} can hand them back rather than just dropping the
     * binder: OsmAnd holds our callback until it is told to stop, and a driver
     * who switches the feature off should stop costing OsmAnd a transaction per
     * location fix.
     *
     * <p>ANY NEGATIVE VALUE MEANS "NOT SUBSCRIBED", AND -1 IS NOT THE ONLY ONE.
     * {@code OsmandAidlServiceV2}'s registration methods return -1 when
     * {@code getApi} refuses the caller, but their catch returns
     * {@code OsmandAidlConstants.UNKNOWN_API_ERROR}, which that file defines as
     * {@code -2} — a registration that THREW inside OsmAnd therefore lands here
     * looking like an ordinary id. Testing for exactly -1 made that channel dead
     * for the rest of the run: the refusal test in {@link #subscribe} was false,
     * the recovery test in {@link #afterPoll} was false so nothing ever retried
     * it, and {@link #stop} sent an unsubscribe for id -2. Every test on these
     * two is therefore {@code < 0}, never {@code == -1}.
     */
    private static long navCallbackId = -1L;
    private static long voiceCallbackId = -1L;

    /**
     * Whether the refused state has been reported, so the 1 Hz poll notes the
     * EDGES and not every tick — the same restraint the poll's own catch shows.
     */
    private static volatile boolean refusedReported;

    /**
     * Whether a poll fault has been reported, edge-triggered like the refusal.
     *
     * <p>THE POLL'S CATCH WAS SILENT AND IT COST A DRIVE. The eager-unparcel
     * fault threw once a second into logcat, which this unit cannot show, and
     * the drive log had nothing to say about a feed that was dead with the
     * permission gate open. One line per edge is the fix; per-tick logging
     * would fill the 600-line ring in ten minutes.
     */
    private static volatile boolean pollFaultReported;

    /**
     * The backoff on the missing-channel re-subscribe: when the next attempt may
     * run ({@link SystemClock#uptimeMillis}), how long the current gap is, and
     * whether the outage has been noted.
     *
     * <p>A GAP THAT DOUBLES, BECAUSE THE RETRY IS NOT FREE. The recovery in
     * {@link #afterPoll} used to re-run {@link #subscribe} on every answered
     * poll whenever EITHER id was missing, and {@code subscribe} re-registers
     * BOTH channels. With one channel stuck that is one re-registration a
     * second, and upstream mints a NEW callback id per call — {@code
     * navUpdateCallbacks.put(id, listener)} plus {@code
     * routingHelper.addRouteDataListener(listener)}, never removed — so a drive
     * accumulated one live listener per second and every node crossing fired
     * {@link IOsmAndAidlCallback#updateNavigationInfo} N times with N growing
     * without bound, while {@link #stop} could hand back only the last pair of
     * ids. It also wrote one {@code safeNote} a second, wiping the 600-line ring
     * every ten minutes — the exact failure {@link #pollFaultReported} exists to
     * prevent. So: only the missing channel is re-registered, the attempt is
     * spaced by this backoff, and the note is edge-triggered.
     *
     * <p>{@code uptimeMillis} AND NOT WALL TIME, because the head unit sets its
     * clock from GPS mid-drive and a jump backwards would freeze the retry for
     * however far it jumped. It is also the clock {@link Handler#postDelayed}
     * schedules the poll on, so the two agree.
     *
     * <p>Written and read under the class lock, like the ids whose repair they
     * pace.
     */
    private static long resubscribeAt;
    private static long resubscribeGap;
    private static boolean resubscribeReported;

    /** The first gap after a failed re-subscribe, and the ceiling it doubles to. */
    private static final long RESUBSCRIBE_FIRST_MS = 5000L;
    private static final long RESUBSCRIBE_MAX_MS = 120000L;

    private CarnyxNav() {
    }

    /** Three integers, exactly as they arrived. See {@code src/nav.rs}. */
    private static native void nativeNav(int distanceTo, int turnType, boolean leftSide);

    /**
     * OsmAnd is refusing this app (true), or has started answering (false).
     *
     * <p>THE ONE LINK STATE A GRAY TELL CANNOT EXPLAIN. Bound-but-refused looks
     * identical to bound-but-idle from the face, and the settings row has to
     * tell the driver which one they are in — one of them has a fix.
     */
    private static native void nativeNavRefused(boolean refused);

    /**
     * The voice router's two lists, unjoined and unfiltered.
     *
     * <p>BOTH OF THEM, because which one to show is a decision and decisions do
     * not live here. {@code cmds} is what the router queued; {@code played} is
     * what the engine said, and it comes back empty when the driver has muted
     * the voice — so the pair is the fact and picking between them is
     * {@code Nav::speak}'s.
     */
    private static native void nativeNavVoice(String[] cmds, String[] played);

    /**
     * The POLL's answer, field by field. See {@code AppInfoParams}.
     *
     * <p>TEN ARGUMENTS AND NO OBJECT, because an object would need a shape and a
     * shape is a decision. Every one of these is passed exactly as OsmAnd gave
     * it — including the zeros that mean "not navigating" and the nulls that mean
     * "this route has no street name" — and `crate::nav` decides what any of it
     * means.
     */
    private static native void nativeNavInfo(
        long arrivalTime, int leftTime, int leftDistance, boolean mapVisible,
        String turnName, String turnType, int turnDistance, int turnImminent,
        String afterName, String afterType);

    /** One line into the diagnostics panel; the unit has no adb. */
    private static native void nativeNavNote(String line);

    /**
     * How often to poll, in milliseconds.
     *
     * <p>ONE SECOND, which is the handoff's "~1 Hz while navigating". The poll is
     * ONE binder round trip returning a small bundle. Faster would buy nothing —
     * OsmAnd recomputes on location fixes, which are 1 Hz on this hardware.
     *
     * <p>THIS COMMENT USED TO ADD THAT THE PUSH CALLBACK "ALREADY ARRIVES AT
     * ABOUT THIS RATE OFF OSMAND'S ROUTING UPDATES, SO THIS ADDS A COMPARABLE
     * COST AND NO MORE", AND THAT WAS FALSE. {@code src/nav.rs:361-368} records
     * the claim as the cause of the blinking maneuver layer: upstream fires
     * {@code IRoutingDataUpdateListener} from ONE site,
     * {@code RoutingHelper.java:730}, inside the {@code if (processed)}
     * node-advance branch of {@code updateCurrentRouteStatus} — i.e. when the
     * vehicle crosses a route geometry node, which on a long straight is minutes
     * apart. The push's rate is unknown and bursty; this one is not, which is
     * why the poll is the channel the face leans on.
     */
    private static final long POLL_MS = 1000L;

    /**
     * The poll's own thread, because a binder call must not run on the UI thread.
     *
     * <p>A `HandlerThread` rather than a timer on the main looper: `getAppInfo` is
     * a synchronous round trip into another app, and another app's slow frame
     * would become our dropped frame. `CarnyxLocation` posts to the main looper
     * for the opposite reason — it REGISTERS listeners, which needs a looper and
     * does not block.
     */
    private static HandlerThread pollThread;

    /**
     * VOLATILE, because {@link Poll} reads it off the poll thread to reschedule
     * itself while {@link #startPoll}/{@link #stopPoll} write it under the class
     * lock. It was a plain static read outside any lock, which is a data race
     * even before the doubling {@link #pollGen} describes.
     */
    private static volatile Handler pollHandler;

    /**
     * Restart guard for the poll, the same one {@code NwdBridge.levelGen}
     * (NwdBridge.java:1198-1204) uses on the level watch, and for the same
     * reason — that comment calls the bare-boolean approach broken.
     *
     * <p>WITHOUT IT THE POLL DOUBLES ON EVERY TOGGLE. {@link #stopPoll} nulls
     * the handler and quits the looper while the outgoing poll is mid-{@code
     * run()}; {@link #startPoll} then builds a new {@code HandlerThread} and
     * posts a poll to it; the outgoing one finishes its iteration, reads the NEW
     * handler and posts a second poll onto it. Two messages then re-post each
     * other for the rest of the run — 2 Hz permanently, and doubling again on
     * the next switch-off/switch-on or OsmAnd death-and-reconnect. Each {@link
     * Poll} captures the generation it was started for and stops rescheduling
     * once it is no longer the current one.
     */
    private static volatile int pollGen;

    /** Hand the class the app context, as {@link CarnyxProcess#attach} does. */
    public static synchronized void attach(Context context) {
        if (ctx == null && context != null) {
            ctx = context.getApplicationContext();
        }
    }

    /**
     * Which OsmAnd is installed, or {@code ""}.
     *
     * <p>Separated from {@link #start} so the settings panel can say whether the
     * feature has anything to talk to WITHOUT binding — a driver reading the
     * switch should not have to turn it on to find out that OsmAnd is missing.
     */
    public static synchronized String installedPackage() {
        if (ctx == null) {
            return "";
        }
        PackageManager pm = ctx.getPackageManager();
        for (String pkg : OSMAND_PACKAGES) {
            Intent intent = new Intent(OSMAND_SERVICE_V2).setPackage(pkg);
            try {
                if (pm.resolveService(intent, 0) != null) {
                    return pkg;
                }
            } catch (Throwable t) {
                Log.w(TAG, "resolveService failed for " + pkg, t);
            }
        }
        return "";
    }

    /**
     * Bind OsmAnd and subscribe.
     *
     * <p>@return a line for the diagnostics log saying what happened, never null.
     *     Binding is ASYNCHRONOUS, so a success here means the request was
     *     accepted and not that the binder arrived; the subscription happens in
     *     {@link #CONN} and reports itself separately.
     */
    public static synchronized String start() {
        if (ctx == null) {
            return "no context — attach() has not run";
        }
        if (osmand != null) {
            return "already bound to " + boundPackage;
        }
        if (binding) {
            return "already binding";
        }
        String pkg = installedPackage();
        if (pkg.isEmpty()) {
            // NOT AN ERROR, and worded so it does not read as one. A driver
            // without OsmAnd has switched on a feature for an app they do not
            // have, which is a thing to tell them rather than a fault.
            return "OsmAnd is not installed — nothing to connect to";
        }
        Intent intent = new Intent(OSMAND_SERVICE_V2).setPackage(pkg);
        try {
            binding = true;
            // BIND_AUTO_CREATE starts OsmAnd if it is not running. That is the
            // intent: a driver who turns this on wants directions, and OsmAnd
            // with no route simply sends nothing.
            if (!ctx.bindService(intent, CONN, Context.BIND_AUTO_CREATE)) {
                binding = false;
                try { ctx.unbindService(CONN); } catch (Throwable ignored) { }
                return "bindService REFUSED by " + pkg;
            }
            boundPackage = pkg;
            return "binding to " + pkg;
        } catch (Throwable t) {
            binding = false;
            return "bindService threw: " + why(t);
        }
    }

    /**
     * Unsubscribe and let go. Safe to call when nothing was started.
     *
     * <p>THE UNSUBSCRIBE IS BEST-EFFORT AND THE UNBIND IS NOT. If OsmAnd has
     * died the first will throw and the second still has to happen, or this
     * process leaks a ServiceConnection for the rest of its life.
     */
    public static synchronized String stop() {
        String outcome = "not connected";
        IOsmAndAidlInterface api = osmand;
        if (api != null) {
            try {
                // >= 0, NOT != -1: a registration that threw inside OsmAnd is
                // stored as -2 and is not an id to hand back. See the field.
                if (navCallbackId >= 0L) {
                    ANavigationUpdateParams p = new ANavigationUpdateParams();
                    p.setCallbackId(navCallbackId);
                    p.setSubscribeToUpdates(false);
                    api.registerForNavigationUpdates(p, CALLBACK);
                }
                if (voiceCallbackId >= 0L) {
                    ANavigationVoiceRouterMessageParams p = new ANavigationVoiceRouterMessageParams();
                    p.setCallbackId(voiceCallbackId);
                    p.setSubscribeToUpdates(false);
                    api.registerForVoiceRouterMessages(p, CALLBACK);
                }
                outcome = "unsubscribed from " + boundPackage;
            } catch (Throwable t) {
                outcome = "unsubscribe failed: " + why(t);
            }
        }
        stopPoll();
        navCallbackId = -1L;
        voiceCallbackId = -1L;
        // The next session starts its retry clock from scratch rather than
        // inheriting a two-minute gap this one had backed off to.
        resubscribeGap = 0L;
        resubscribeReported = false;
        osmand = null;
        binding = false;
        boundPackage = "";
        try {
            ctx.unbindService(CONN);
        } catch (Throwable ignored) {
            // Unbinding a connection that was never bound is not a fault.
        }
        return outcome;
    }

    /**
     * Bring OsmAnd to the front — the tap the status-bar mark answers.
     *
     * <p>WORKS WHETHER OR NOT THE FEED IS BOUND. A driver may tap the mark
     * while it is still dim (enabled but not yet answering) or even before the
     * poll has run once, so this resolves the package itself rather than
     * trusting {@link #boundPackage}, which is only set once binding has
     * actually succeeded.
     *
     * <p>{@code FLAG_ACTIVITY_NEW_TASK} IS NOT OPTIONAL. {@link #ctx} is the
     * application context ({@link #attach}), and {@code startActivity} off a
     * non-Activity context throws {@code AndroidRuntimeException} without it.
     * No other flag is needed for "switch to, not relaunch": OsmAnd's own
     * launcher activity is standard launch mode, so Android brings its
     * existing task to the front by itself when one is already running —
     * the same behaviour tapping its icon in a launcher would produce.
     */
    public static synchronized String launch() {
        if (ctx == null) {
            return "no context — attach() has not run";
        }
        String pkg = boundPackage.isEmpty() ? installedPackage() : boundPackage;
        if (pkg.isEmpty()) {
            return "OsmAnd is not installed — nothing to launch";
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
     * One poll, then schedule the next.
     *
     * <p>RESCHEDULES ITSELF EVEN WHEN THE CALL FAILS. A `DeadObjectException`
     * means OsmAnd went away, and `onServiceDisconnected` handles that — this
     * must not also decide to stop, or a transient failure would silence the
     * poll for the rest of the run with nothing recorded.
     *
     * <p>BUT ONLY WHILE IT IS STILL THE CURRENT POLL. One instance per
     * {@link #startPoll}, each carrying the generation it was started for; an
     * outgoing one that finishes its iteration after a restart sees a newer
     * {@link #pollGen} and lets itself end instead of seeding a second chain on
     * the new looper. See that field for the doubling this prevents.
     */
    private static final class Poll implements Runnable {
        private final int gen;

        Poll(int gen) {
            this.gen = gen;
        }

        @Override
        public void run() {
            IOsmAndAidlInterface api;
            synchronized (CarnyxNav.class) {
                api = osmand;
            }
            if (api != null) {
                // TRI-STATE ON PURPOSE. A throw is a transport fault and says
                // nothing about permission — marking it refused would flash the
                // wrong instruction at the driver over a blip. Only a clean
                // null answer means refused; only a real answer means accepted.
                Boolean answered = null;
                try {
                    answered = pollOnce(api);
                } catch (Throwable t) {
                    Log.w(TAG, "getAppInfo failed", t);
                    // ONE LINE PER EDGE, WITH THE REASON. The fault that taught
                    // this lesson was a BadParcelableException thrown on every
                    // poll of a drive — visible nowhere, because this catch
                    // logged only to logcat and the unit has no adb.
                    if (!pollFaultReported) {
                        pollFaultReported = true;
                        safeNote("nav: getAppInfo failing — " + why(t));
                    }
                }
                if (answered != null) {
                    if (pollFaultReported) {
                        pollFaultReported = false;
                        safeNote("nav: getAppInfo recovered");
                    }
                    afterPoll(answered);
                }
            }
            // THE GENERATION IS CHECKED AT THE POST, not at the top of the run:
            // the restart can land at any point inside the iteration above, and
            // it is the re-post that does the damage.
            Handler h = pollHandler;
            if (h != null && gen == pollGen) {
                h.postDelayed(this, POLL_MS);
            }
        }
    }

    /**
     * Ask once and hand every field across.
     *
     * @return whether OsmAnd ANSWERED. Null is not "no route" — an idle OsmAnd
     *     answers with zeroed fields — it is `getApi` refusing the caller, and
     *     the poll's follow-up turns that into the refused state.
     */
    private static boolean pollOnce(IOsmAndAidlInterface api) throws RemoteException {
        AppInfoParams info = api.getAppInfo();
        if (info == null) {
            return false;
        }
        Bundle turn = info.getTurnInfo();
        nativeNavInfo(
            // *1000L, AND THIS IS THE FIX, NOT DECORATION. `AppInfoParams`'s own
            // javadoc calls this "Unix millis" — wrong. Upstream computes it as
            // `arrivalTime = leftTime + System.currentTimeMillis() / 1000` (see
            // `OsmandAidlApi.getAppInfo`), which is Unix SECONDS. Everything past
            // this line — the native parameter name, `Route.arrival_ms`,
            // `crate::clock::eta`'s own `div_euclid(1000)` — was built and tested
            // assuming milliseconds, so the conversion belongs HERE, at the one
            // seam that knows the wire's real unit, not scattered downstream.
            // Unconverted, a ~1.7-billion-second value divided by 1000 a second
            // time lands on a date three weeks after the epoch — a bogus
            // time-of-day that barely moves for the length of a drive (the raw
            // seconds value is itself nearly constant: `leftTime` falls as `now`
            // rises), which reads on the face as exactly what a drive reported:
            // a static ETA, wrong by hours.
            info.getArrivalTime() * 1000L,
            info.getLeftTime(),
            info.getLeftDistance(),
            info.isMapVisible(),
            str(turn, "next_turn_name"),
            str(turn, "next_turn_type"),
            turn == null ? 0 : turn.getInt("next_turn_distance"),
            // NOT A BOOLEAN. OsmAnd writes `nextInfo.imminent`, an int, and what
            // its values mean is not established anywhere this tree could read —
            // the class that computes it has moved out of the Java sources. It
            // crosses raw and `crate::nav` logs it, so one drive settles the
            // scale instead of a guess shipping.
            turn == null ? -1 : turn.getInt("next_turn_imminent", -1),
            // THE AFTER-NEXT PREFIX HAS NO UNDERSCORE, and that is upstream's,
            // not a typo here. `ExternalApiHelper` calls
            // `updateTurnInfo("after_next", bundle, ni)` where the next turn uses
            // `"next_"`, and the keys are built by concatenation — so the key is
            // literally `after_nextturn_name`. Spelling it the tidy way would
            // read back null and collapse the THEN block with nothing to say why.
            str(turn, "after_nextturn_name"),
            str(turn, "after_nextturn_type"));
        return true;
    }

    /** A bundle string, or null. Null is "this route has none", not an error. */
    private static String str(Bundle b, String key) {
        return b == null ? null : b.getString(key);
    }

    private static final ServiceConnection CONN = new ServiceConnection() {
        @Override
        public void onServiceConnected(ComponentName name, IBinder service) {
            String note;
            synchronized (CarnyxNav.class) {
                binding = false;
                osmand = IOsmAndAidlInterface.Stub.asInterface(service);
                note = subscribe();
            }
            safeNote("nav: " + note);
        }

        @Override
        public void onServiceDisconnected(ComponentName name) {
            synchronized (CarnyxNav.class) {
                stopPoll();
                osmand = null;
                navCallbackId = -1L;
                voiceCallbackId = -1L;
                // The reconnect subscribes afresh, so it starts its retry clock
                // afresh too rather than inheriting this session's backoff.
                resubscribeGap = 0L;
                resubscribeReported = false;
            }
            // THE FACE HAS TO BE TOLD, because the updates simply stop and
            // `Nav::EXPIRY` would clear the turn twelve seconds later with no
            // reason recorded anywhere.
            safeNote("nav: OsmAnd disconnected");
        }
    };

    /**
     * Subscribe to both streams. Called with the lock held.
     *
     * <p>BOTH, because neither is enough alone: {@code ADirectionInfo} is the
     * turn and the distance with no words in it at all, and the voice router is
     * the words with no distance. A face that shows "in 240 m" needs the first
     * and a face that shows a street name needs the second.
     *
     * <p>THIS IS THE FIRST-CONNECT PATH ONLY. The recovery path re-registers one
     * channel at a time through {@link #subscribeNav}/{@link #subscribeVoice} —
     * see {@link #mendSubscriptions} for why calling this again would be a leak.
     */
    private static String subscribe() {
        IOsmAndAidlInterface api = osmand;
        if (api == null) {
            return "connected, but the binder was null";
        }
        StringBuilder out = new StringBuilder("connected to ").append(boundPackage);
        subscribeNav(api, out);
        subscribeVoice(api, out);
        // BOTH IDS NEGATIVE IS OSMAND SAYING NO, not a registration quirk. Per
        // upstream's own source there is one common way to get here: `getApi`
        // found this package in the connected-apps list switched off — where
        // OsmAnd itself put it, disabled, on our first call. The fix is the
        // driver's one-time toggle, so the log says exactly where it is.
        //
        // < 0 AND NOT == -1: a refusal returns -1, but a throw inside OsmAnd
        // returns UNKNOWN_API_ERROR, which is -2. See the id fields.
        if (navCallbackId < 0L && voiceCallbackId < 0L) {
            if (!refusedReported) {
                refusedReported = true;
                safeRefused(true);
            }
            out.append(" — OsmAnd is refusing this app; open OsmAnd's Plugins screen")
               .append(" and switch on Carnyx (it appears there after this attempt)");
        }
        startPoll();
        return out.toString();
    }

    /**
     * Register the TURNS channel and say what happened. Called with the lock held.
     *
     * <p>ITS OWN METHOD SO THE RECOVERY CAN CALL ONE HALF. Every call mints a
     * fresh callback id upstream and adds a fresh route-data listener. A
     * listener CAN be removed — that is what {@link #stop} does, re-calling
     * this same AIDL method with the id and {@code subscribeToUpdates(false)},
     * which reaches upstream's {@code unregisterFromUpdates(id)}. But removing
     * one needs its id, and {@link #navCallbackId} only ever holds the LATEST:
     * re-registering a channel that is already live overwrites the id of the
     * listener still attached and leaves it running with nothing able to name
     * it again. So a live channel must not be re-registered — see
     * {@link #mendSubscriptions}.
     *
     * <p>THE ID IS CLEARED ON A THROW rather than left as it was: no id came
     * back, so there is nothing for {@link #stop} to hand OsmAnd, and the
     * recovery has to see this channel as missing.
     */
    private static void subscribeNav(IOsmAndAidlInterface api, StringBuilder out) {
        try {
            ANavigationUpdateParams p = new ANavigationUpdateParams();
            p.setSubscribeToUpdates(true);
            navCallbackId = api.registerForNavigationUpdates(p, CALLBACK);
            out.append(", turns id ").append(navCallbackId);
        } catch (Throwable t) {
            navCallbackId = -1L;
            out.append(", TURNS FAILED: ").append(why(t));
        }
    }

    /** Register the VOICE channel and say what happened. See {@link #subscribeNav}. */
    private static void subscribeVoice(IOsmAndAidlInterface api, StringBuilder out) {
        try {
            ANavigationVoiceRouterMessageParams p = new ANavigationVoiceRouterMessageParams();
            p.setSubscribeToUpdates(true);
            voiceCallbackId = api.registerForVoiceRouterMessages(p, CALLBACK);
            out.append(", voice id ").append(voiceCallbackId);
        } catch (Throwable t) {
            voiceCallbackId = -1L;
            out.append(", VOICE FAILED: ").append(why(t));
        }
    }

    /**
     * The callback OsmAnd holds. Every method arrives on a BINDER THREAD.
     *
     * <p>Seven of the nine do nothing and are not stubs standing in for work —
     * they are slots this app never subscribed to, and OsmAnd will not call
     * them. They exist because the interface declares them and because a
     * transaction id is positional: an implementation missing a method would
     * shift nothing, but a DECLARATION missing one would.
     */
    private static final IOsmAndAidlCallback.Stub CALLBACK = new IOsmAndAidlCallback.Stub() {
        @Override public void onSearchComplete(List<SearchResult> resultSet) { }

        @Override public void onUpdate() { }

        @Override public void onAppInitialized() { }

        @Override public void onGpxBitmapCreated(AGpxBitmap bitmap) { }

        @Override
        public void updateNavigationInfo(ADirectionInfo directionInfo) {
            if (directionInfo == null) {
                return;
            }
            // STRAIGHT ACROSS. `-1, -1` is passed on exactly as it arrived —
            // OsmAnd's own "navigating, nothing to say" — because turning it
            // into a state here would be the decision this class does not make.
            try {
                nativeNav(
                    directionInfo.getDistanceTo(),
                    directionInfo.getTurnType(),
                    directionInfo.isLeftSide());
            } catch (Throwable t) {
                Log.w(TAG, "nativeNav failed", t);
            }
        }

        @Override
        public void onContextMenuButtonClicked(int buttonId, String pointId, String layerId) { }

        @Override
        public void onVoiceRouterNotify(OnVoiceNavigationParams params) {
            if (params == null) {
                return;
            }
            try {
                nativeNavVoice(toArray(params.getCommands()), toArray(params.getPlayed()));
            } catch (Throwable t) {
                Log.w(TAG, "nativeNavVoice failed", t);
            }
        }

        @Override public void onKeyEvent(android.view.KeyEvent params) { }

        @Override public void onLogcatMessage(OnLogcatMessageParams params) { }
    };

    /** Never null, never holding a null element — JNI reads it element by element. */
    private static String[] toArray(List<String> list) {
        if (list == null || list.isEmpty()) {
            return new String[0];
        }
        String[] out = new String[list.size()];
        for (int i = 0; i < out.length; i++) {
            String s = list.get(i);
            out[i] = s == null ? "" : s;
        }
        return out;
    }

    /** Start the 1 Hz poll. Idempotent. */
    private static synchronized void startPoll() {
        if (pollHandler != null) {
            return;
        }
        // THE GENERATION IS TAKEN AFTER THE IDEMPOTENCE GUARD, so a second call
        // that changes nothing cannot invalidate the poll that is already
        // running. See `pollGen`.
        int gen = ++pollGen;
        pollThread = new HandlerThread("carnyx-osmand-poll");
        pollThread.start();
        pollHandler = new Handler(pollThread.getLooper());
        pollHandler.post(new Poll(gen));
    }

    /** Stop it and let the thread go. Safe when it was never started. */
    private static synchronized void stopPoll() {
        // BUMPED FIRST, AND UNCONDITIONALLY. `removeCallbacksAndMessages` clears
        // the QUEUE, which does nothing about an iteration already running on
        // the poll thread — that one is stopped here, by making its generation
        // stale before it reaches its re-post.
        pollGen++;
        Handler h = pollHandler;
        pollHandler = null;
        if (h != null) {
            h.removeCallbacksAndMessages(null);
        }
        HandlerThread t = pollThread;
        pollThread = null;
        if (t != null) {
            t.quit();
        }
    }

    /**
     * What one answered (or refused) poll means for the link state.
     *
     * <p>THE POLL IS THE RECOVERY PATH as well as the detector. When the driver
     * flips this app on inside OsmAnd, nothing calls us — OsmAnd just starts
     * answering — so the first real answer after a refusal clears the state
     * AND re-registers whichever subscription came back negative while refused.
     * Without that, enabling Carnyx would light the tell and leave the push
     * callbacks dead until the next toggle or reboot.
     */
    private static void afterPoll(boolean answered) {
        if (!answered) {
            if (!refusedReported) {
                refusedReported = true;
                safeRefused(true);
                safeNote("nav: OsmAnd is refusing this app — open OsmAnd's"
                        + " Plugins screen and switch on Carnyx");
            }
            return;
        }
        if (refusedReported) {
            refusedReported = false;
            safeRefused(false);
        }
        String note;
        synchronized (CarnyxNav.class) {
            note = mendSubscriptions();
        }
        if (note != null) {
            safeNote("nav: " + note);
        }
    }

    /**
     * Re-register whichever channel is missing. Called with the lock held.
     *
     * <p>ONE CHANNEL, NOT BOTH, AND NOT EVERY TICK. This used to call
     * {@link #subscribe}, which re-registers BOTH channels, whenever EITHER id
     * was missing, with no latch and no backoff. One stuck channel therefore
     * re-registered the healthy one once a second for the rest of the session,
     * and upstream mints a NEW id per call — {@code navUpdateCallbacks.put(id,
     * listener)} plus {@code routingHelper.addRouteDataListener(listener)},
     * never removed. So the listener count grew by one a second, every node
     * crossing fired {@code updateNavigationInfo} that many times, and
     * {@link #stop} could hand back only the last pair of ids. The
     * once-a-second note that went with it wiped the 600-line diagnostics ring
     * every ten minutes, which on a unit with no adb is the whole channel.
     *
     * <p>THE NOTE IS EDGE-TRIGGERED, like {@link #pollFaultReported}: one line
     * when the outage opens, one when a retry mends it, nothing in between.
     *
     * <p>IT DOES NOT TOUCH THE REFUSED STATE, and {@link #subscribe}'s copy of
     * that test is deliberately not carried over here. This runs only on an
     * ANSWERED poll, which is {@code getApi} NOT refusing us; a registration
     * that fails anyway is a different fault, and lighting the refused tell
     * would send the driver to toggle a switch that is already on — right after
     * {@link #afterPoll} cleared the tell two lines earlier.
     *
     * @return a line for the log, or null when there is nothing new to say —
     *     which is the usual answer, once per second, for the whole of a drive.
     */
    private static String mendSubscriptions() {
        IOsmAndAidlInterface api = osmand;
        boolean navMissing = api != null && navCallbackId < 0L;
        boolean voiceMissing = api != null && voiceCallbackId < 0L;
        if (!navMissing && !voiceMissing) {
            // Nothing to mend. The gap is cleared so a channel lost LATER is
            // retried at once instead of inheriting this outage's backoff.
            resubscribeGap = 0L;
            resubscribeReported = false;
            return null;
        }
        long now = SystemClock.uptimeMillis();
        if (resubscribeGap != 0L && now - resubscribeAt < 0L) {
            return null;
        }
        resubscribeGap = resubscribeGap == 0L
            ? RESUBSCRIBE_FIRST_MS
            : Math.min(resubscribeGap * 2L, RESUBSCRIBE_MAX_MS);
        resubscribeAt = now + resubscribeGap;
        StringBuilder out = new StringBuilder("re-subscribing to ").append(boundPackage);
        if (navMissing) {
            subscribeNav(api, out);
        }
        if (voiceMissing) {
            subscribeVoice(api, out);
        }
        boolean mended = (navMissing && navCallbackId >= 0L)
            || (voiceMissing && voiceCallbackId >= 0L);
        if (navCallbackId >= 0L && voiceCallbackId >= 0L) {
            // Everything is live again: forget the backoff so a later loss is
            // retried at once, and reopen the note for whatever that loss is.
            resubscribeGap = 0L;
            resubscribeReported = false;
            // A MEND IS ALWAYS WORTH A LINE, even when the outage never got one:
            // this is what says the driver's toggle inside OsmAnd took.
            return out.toString();
        }
        if (mended) {
            // Half of it took, and the line names which half is still dead. It
            // can happen at most once per outage — one channel is now live and
            // only the other can mend after it — so the ring stays safe.
            resubscribeReported = true;
            return out.toString();
        }
        if (resubscribeReported) {
            return null;
        }
        resubscribeReported = true;
        return out.append("; still missing — next try in ")
                  .append(resubscribeGap / 1000L)
                  .append(" s, doubling to a ")
                  .append(RESUBSCRIBE_MAX_MS / 1000L)
                  .append(" s ceiling")
                  .toString();
    }

    /** The refused edge, delivered so it cannot break the poll. */
    private static void safeRefused(boolean refused) {
        try {
            nativeNavRefused(refused);
        } catch (Throwable t) {
            Log.w(TAG, "refused state not delivered", t);
        }
    }

    /** A note that cannot itself break the caller. See {@code NwdBridge.safe*}. */
    private static void safeNote(String line) {
        try {
            nativeNavNote(line);
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

    /** Unused import guard: `RemoteException` is what the AIDL stubs declare. */
    @SuppressWarnings("unused")
    private static void declaredThrows() throws RemoteException {
    }
}
