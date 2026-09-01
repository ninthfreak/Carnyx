package com.ninthfreak.carnyx;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.widget.RemoteViews;
import android.widget.Toast;

/**
 * The station pop-up: a heads-up notification saying what is now tuned, for the
 * driver who changed station from the steering wheel while looking at another
 * app.
 *
 * <h2>Why this is not in the service</h2>
 *
 * <p>{@link CarnyxService} exists to pin the process and is in the GRADLE source
 * set, which means it does not exist at all under cargo-apk. Posting a
 * notification needs none of that — any component holding a {@link Context} can
 * do it — so this lives in {@code java/}, the runtime dex, beside
 * {@link CarnyxProcess}. Both packagers get it.
 *
 * <p>It also sidesteps a real trap. Reaching the service would mean
 * {@code startService} from the BACKGROUND, which API 26 forbids outright and
 * which API 31 forbids again for {@code startForegroundService} — and the
 * background is the only time this feature fires. Posting directly has no such
 * rule.
 *
 * <h2>The channel is its own, and loud on purpose</h2>
 *
 * <p>The service's channel is {@code IMPORTANCE_LOW}: an ongoing "this is
 * running" line with no sound and no heads-up, which is what it honestly is.
 * This one is {@code IMPORTANCE_HIGH}, because a heads-up banner IS the feature
 * — a station confirmation the driver never sees is not a confirmation. Two
 * channels rather than one raised, so a driver who wants the pop-up but not the
 * ongoing line, or the reverse, can say so in Settings and the app obeys without
 * knowing.
 *
 * <h2>ON THIS ROM THE NOTIFICATION IS NOT WHAT THE DRIVER SEES</h2>
 *
 * <p>Posting works and shows nothing. A drive produced {@code station pop-up:
 * WQLF at 102.1 (logo) — posted, channel importance 4} twice, with
 * {@code panel key … [background]} above each, and the owner saw no banner and
 * no shade entry. That rules out every failure this class can name — the gate,
 * the context, {@code areNotificationsEnabled}, a downgraded channel, a throw —
 * and leaves SystemUI, which on a head unit of this class often has no heads-up
 * banner and sometimes no notification panel at all.
 *
 * <p>So {@link #post} ALSO RAISES A TOAST, which is a WindowManager window
 * rather than SystemUI's, and needs no channel, no importance and no shade. The
 * notification is still posted: it costs nothing where it is invisible and is
 * the better artefact where a shade exists. See {@link #toast}.
 *
 * <h2>What the driver's Android version decides</h2>
 *
 * <p>ON THE UNIT THIS IS BUILT FOR — Android 10, API 29 — posting needs no
 * permission at all and this simply works. {@code POST_NOTIFICATIONS} is an
 * API 33 runtime permission, and on a newer head unit an ungranted one makes the
 * platform DROP the notification silently. That is the one failure worth being
 * loud about internally, so {@link #post} checks
 * {@code areNotificationsEnabled()} and says so in the diagnostics log rather
 * than returning as though it had worked. This app does not raise the runtime
 * dialog — the manifest's standing note applies, that a request "needs someone to
 * tap Allow, which on a dashboard at night is nobody" — so on such a unit the
 * driver grants it in Settings or does without.
 */
public final class CarnyxAlert {
    private static final String TAG = "CarnyxAlert";

    /** Distinct from the service's channel and its id; see the class note. */
    private static final String CHANNEL_ID = "carnyx.station";
    private static final int NOTIFICATION_ID = 2;

    /**
     * How long the line lingers after the banner has gone.
     *
     * <p>A station confirmation is stale the moment the next one arrives, and a
     * shade filling with them is litter. The banner itself is the platform's to
     * time; this is the entry behind it, and {@code setTimeoutAfter} is the only
     * way to have the platform clear one without the app being alive to do it.
     */
    private static final long TIMEOUT_MS = 8000;

    /**
     * Roughly what the platform draws a large icon at, in dp. Used only to pick
     * a decode step; the platform does the final scaling either way.
     */
    private static final float ICON_DP = 64f;

    /**
     * The toast's type size, in sp.
     *
     * <p>28 rather than the platform's 14, because the first one on the unit came
     * back as <i>"a tiny pop-up at the bottom of the screen"</i>. This is read
     * from the driver's seat, at a glance, on a dashboard — the same brief as the
     * face's own dial, which is far larger still. sp and not dp so a raised
     * system font size raises this too.
     */
    private static final float TOAST_SP = 28f;

    /** The toast's padding and corner, in dp. See {@link #toastView}. */
    private static final float TOAST_PAD_X_DP = 28f;
    private static final float TOAST_PAD_Y_DP = 18f;
    private static final float TOAST_RADIUS_DP = 18f;

    /**
     * The custom banner's layout, looked up by NAME rather than through {@code R}.
     *
     * <p>THIS CLASS IS IN THE RUNTIME DEX, compiled by {@code build.rs} against
     * {@code android.jar} alone, and {@code R} is generated by AGP — it is not on
     * that compile path and never can be. {@code getIdentifier} asks the package
     * manager at run time instead, which is the one lookup that works from either
     * tree.
     *
     * <p>ZERO IS THE cargo-apk BUILD, and an expected answer rather than a fault:
     * that packager ships no resources at all, so the layout is genuinely absent
     * and {@link #build} falls back to the platform's own template. Resolved once
     * and cached — {@code getIdentifier} is a string lookup through the resource
     * table, and this runs on every station change.
     */
    private static int layoutId = -1;
    private static int callId;
    private static int dialId;

    private static Context ctx;
    private static boolean channelMade;

    /**
     * The Activity, kept ONLY so a permission can be asked for. See
     * {@link #requestPostNotifications}.
     *
     * <p>A WEAK REFERENCE, because this is a static field on a class that lives
     * as long as the process and an Activity does not. A strong one would pin a
     * destroyed Activity, its window and its whole view tree for the life of the
     * app — the textbook Android leak, and a real cost on units shipping the RAM
     * these do.
     *
     * <p>{@link #ctx} stays the APPLICATION context and everything else keeps
     * using it. Posting a notification and raising a toast both outlive any one
     * Activity and neither should hold one.
     */
    private static java.lang.ref.WeakReference<android.app.Activity> activity;

    private CarnyxAlert() {
    }

    /**
     * Hand the class the app context, as {@link CarnyxProcess#attach} does.
     *
     * <p>What Rust passes is the {@code NativeActivity} itself — {@code
     * alert::init} hands over the pointer `AndroidApp` gave it — so this takes
     * the application context for its own use AND keeps the Activity weakly,
     * because asking for a runtime permission needs one and an application
     * context cannot do it.
     */
    public static synchronized void attach(Context context) {
        if (context == null) {
            return;
        }
        if (ctx == null) {
            ctx = context.getApplicationContext();
        }
        if (context instanceof android.app.Activity) {
            activity = new java.lang.ref.WeakReference<>((android.app.Activity) context);
        }
    }

    /**
     * Ask the driver for the notification permission, where this Android needs
     * one and does not already have it.
     *
     * <p>WHY THIS EXISTS, given the standing note that a permission dialog
     * "needs someone to tap Allow, which on a dashboard at night is nobody".
     * That note is about asking UNPROMPTED, at start-up, mid-drive, and it still
     * holds — nothing calls this on its own. What it never justified was having
     * no way to ask AT ALL, which left the station pop-up silently dead on any
     * unit running API 33 or newer with no route to fix it from inside the app.
     * This is that route: a row the driver taps deliberately, parked, already
     * looking at the settings panel.
     *
     * <p>THE ANSWERS THAT ARE NOT A DIALOG all come back as a line rather than
     * as silence, because a row that appears to do nothing is the exact failure
     * the five removed diagnostics rows had:
     *
     * <ul>
     *   <li>BELOW API 33 there is no such permission and the manifest
     *       declaration is the whole story. That is the common case for the
     *       low-end Android 8-to-10 units this app is built for, and this one.
     *   <li>ALREADY GRANTED. Asking again shows nothing on modern Android, so
     *       reporting it is the only way the tap is legible.
     *   <li>PERMANENTLY DENIED IS NOT DISTINGUISHABLE FROM HERE and this does
     *       not pretend otherwise. After two refusals Android stops showing the
     *       dialog and {@code requestPermissions} returns having done nothing at
     *       all. So the line says the request went out and that a dialog may not
     *       have appeared, rather than claiming the driver was asked.
     * </ul>
     *
     * <p>NO RESULT CALLBACK IS WIRED, and none is needed.
     * {@code onRequestPermissionsResult} lands on the Activity, which is Slint's
     * {@code NativeActivity} and not ours to override. The next {@link #post}
     * reads {@code areNotificationsEnabled()} and logs what it found, so the
     * answer arrives on the next station change through a channel that already
     * exists.
     *
     * @return one line for the diagnostics log, never null.
     */
    public static synchronized String requestPostNotifications() {
        if (Build.VERSION.SDK_INT < 33) {
            return "notification permission: not needed below API 33 (this unit is API "
                    + Build.VERSION.SDK_INT + ")";
        }
        if (ctx == null) {
            return "notification permission: no context — attach() has not run";
        }
        try {
            String perm = "android.permission.POST_NOTIFICATIONS";
            int granted = android.content.pm.PackageManager.PERMISSION_GRANTED;
            if (ctx.checkSelfPermission(perm) == granted) {
                return "notification permission: already granted";
            }
            android.app.Activity a = activity == null ? null : activity.get();
            if (a == null) {
                return "notification permission: NOT granted, and no Activity to ask with"
                        + " — grant it in Android's own app settings";
            }
            a.requestPermissions(new String[] { perm }, 1);
            return "notification permission: asked. Tap Allow if a dialog appeared;"
                    + " Android stops showing it after two refusals, and then it has"
                    + " to be granted in Android's own app settings";
        } catch (Throwable t) {
            return "notification permission: request failed — " + why(t);
        }
    }

    /**
     * Show what is tuned now.
     *
     * <p>ONE ID, SO REPEATS REPLACE. Stepping four presets with the wheel is one
     * notification updated four times, not four notifications — the driver wants
     * to know where they landed, not where they have been.
     *
     * <p>THE CALL SIGN AND THE DIAL ALWAYS, and the logo beside them when the
     * station has one. A logo-only banner was tried first and does not work —
     * see {@link #build} for what the platform actually does with a landscape
     * wordmark in a square slot.
     *
     * @param title the call sign, or the dial when no call sign has resolved
     * @param text the second line, the dial
     * @param logoPath the station's saved logo, or empty for none. A path that
     *     does not decode simply leaves the banner as words, which is the whole
     *     message either way.
     * @return WHAT HAPPENED, for the diagnostics log — "posted, channel
     *     importance 4, toast queued", "notifications are off for this app",
     *     "notify threw: …". It returned a bool and told logcat the reason,
     *     which on a unit with no adb reaches nobody: every way of failing
     *     printed the same "not posted", and they need different fixes — one is
     *     a driver's Settings toggle, one needs a new channel id, one is
     *     SystemUI's. THE LAST CLAUSE IS THE TOAST, which on this ROM is the
     *     half the driver actually sees; see {@link #toast}.
     */
    public static synchronized String post(String title, String text, String logoPath) {
        if (ctx == null) {
            return "post() before attach()";
        }
        NotificationManager nm = ctx.getSystemService(NotificationManager.class);
        if (nm == null) {
            return "no NotificationManager";
        }
        // API 33+ only. Below it the method exists and answers for the app as a
        // whole, which is still worth honouring: a driver who turned Carnyx's
        // notifications off in Settings has said what they want.
        if (!nm.areNotificationsEnabled()) {
            return "notifications are off for this app";
        }
        ensureChannel(nm);
        String posted;
        try {
            nm.notify(NOTIFICATION_ID, build(title, text, decode(logoPath)));
            posted = "posted, " + channelState();
        } catch (Throwable t) {
            posted = "notify threw: " + why(t);
        }
        return posted + ", " + toast(title, text);
    }

    /**
     * The same message as a TOAST, because on this unit the notification is not
     * what the driver sees.
     *
     * <h2>Why this exists at all</h2>
     *
     * <p>ONE DRIVE SETTLED IT. Two wheel presses with another app in front
     * produced, in the diagnostics log, {@code panel key 62 (preset next) …
     * [background]} followed by {@code station pop-up: WQLF at 102.1 (logo) —
     * posted, channel importance 4}. So the announce gate opened, the app's
     * notifications are enabled, the channel is {@code IMPORTANCE_HIGH}, and
     * {@code notify} did not throw — and the owner saw nothing. Every failure
     * this class can name had been ruled out; what is left is the ROM, which on
     * a head unit of this class is entirely ordinary. SystemUI here raises no
     * heads-up banner.
     *
     * <h2>Why a toast is the answer and not another notification trick</h2>
     *
     * <p>A toast is not SystemUI's. It is a window the WindowManager puts up
     * over whatever is in front, and it needs no channel, no importance, no
     * shade and no notification panel — so every mechanism that swallowed the
     * banner is out of the path. On API 29 a background app may still raise one:
     * the block on background toasts landed in API 30 and covers CUSTOM views,
     * and a text toast is exempt.
     *
     * <h2>Both, not either</h2>
     *
     * <p>The notification is still posted. It costs nothing where it is
     * invisible, and where a shade DOES exist it is the better artefact —
     * tappable, and it comes back to the face. The toast is the one that shows
     * on this unit.
     *
     * <h2>The main looper, not this thread</h2>
     *
     * <p>{@code post} arrives on the NATIVE thread running {@code android_main},
     * which has an {@code ALooper} but no Java {@link Looper} — {@code
     * Toast.show} on it would throw "Can't create handler inside thread that has
     * not called Looper.prepare()". {@code Looper.getMainLooper()} is always
     * there and is where a toast belongs.
     *
     * <h2>BIG, AND IN THE UPPER QUARTER</h2>
     *
     * <p>The first drive with this in it got one: <i>"a tiny pop-up at the bottom
     * of the screen"</i>. A platform text toast is small, grey and bottom-centred,
     * which is right for "copied to clipboard" and wrong for the one thing the
     * driver looked away from the road to find out.
     *
     * <p>So it is drawn rather than defaulted — see {@link #toastView} — and
     * placed with {@code setGravity}. BOTH OF THOSE ARE API 29 CAPABILITIES AND
     * THAT IS WHY THIS UNIT CAN HAVE THEM: API 30 deprecated {@code setView},
     * blocked custom toasts from the background, and made {@code setGravity} a
     * no-op for text toasts. The unit is Android 10. A newer one falls back to a
     * plain text toast, which is small and at the bottom but still says the
     * station — the branch is on {@code Build.VERSION.SDK_INT} and the log line
     * says which was used.
     *
     * @return one clause for the diagnostics log, so the drive after this one
     *     says which of the two the driver was actually shown.
     */
    private static String toast(String title, String text) {
        if (ctx == null) {
            return "no toast: no context";
        }
        final String message = title == null || title.isEmpty()
                ? (text == null ? "" : text)
                : (text == null || text.isEmpty() ? title : title + "  ·  " + text);
        if (message.isEmpty()) {
            return "no toast: nothing to say";
        }
        final boolean custom = Build.VERSION.SDK_INT < Build.VERSION_CODES.R;
        try {
            Looper main = Looper.getMainLooper();
            if (main == null) {
                return "no toast: no main looper";
            }
            boolean queued = new Handler(main).post(new Runnable() {
                @Override public void run() {
                    try {
                        Toast t = Toast.makeText(ctx, message, Toast.LENGTH_LONG);
                        if (custom) {
                            t.setView(toastView(message));
                            // TOP AND CENTRED, one sixteenth of the screen down.
                            // With TOP gravity the offset is from the screen's top
                            // edge to the toast's, so a sixteenth leaves room for a
                            // box three sixteenths tall before it leaves the upper
                            // quarter — and the box is about a fifth of that.
                            t.setGravity(Gravity.TOP | Gravity.CENTER_HORIZONTAL, 0,
                                    ctx.getResources().getDisplayMetrics().heightPixels / 16);
                        }
                        t.show();
                    } catch (Throwable t) {
                        // Nothing to recover and nobody to tell: this runs a
                        // moment later, on another thread, after `post` has
                        // already returned its line to the log.
                        Log.w(TAG, "toast failed: " + why(t));
                    }
                }
            });
            if (!queued) {
                return "toast REFUSED by the main looper";
            }
            return custom ? "toast queued" : "toast queued, platform default (API 30+)";
        } catch (Throwable t) {
            return "toast threw: " + why(t);
        }
    }

    /**
     * The toast's own view, BUILT IN CODE BECAUSE THERE IS NO LAYOUT TO INFLATE.
     *
     * <p>This class is in the RUNTIME DEX, compiled by {@code build.rs} against
     * {@code android.jar} alone — it has no {@code R} and, under cargo-apk, the
     * package has no resources at all. {@link #ensureLayout} solves the same
     * problem for the notification by asking the package manager for a layout by
     * name and doing without when there is none; a toast has no platform
     * template worth falling back to, so this one is assembled from a
     * {@code TextView} and a {@code GradientDrawable}, which need nothing from a
     * resource table.
     *
     * <p>THE SIZES ARE IN dp AND sp, NOT PIXELS. The unit is 1024x614 at its own
     * density and the phone surfaces in the handoff are not, so a box measured in
     * pixels would be a different size on each. {@code COMPLEX_UNIT_SP} on the
     * text also means a driver who has raised the system font size gets a bigger
     * one here, which is the whole reason that unit exists.
     *
     * <p>The colours are the face's own: {@code Pal.blue}'s dark value for the
     * edge, near-black for the ground, white for the words. Deliberately NOT the
     * theme's egg accent — this draws while another app is in front, where a
     * band's colours would be unexplained.
     */
    private static android.view.View toastView(String message) {
        float density = ctx.getResources().getDisplayMetrics().density;
        android.widget.TextView tv = new android.widget.TextView(ctx);
        tv.setText(message);
        tv.setTextColor(0xFFFFFFFF);
        tv.setTextSize(android.util.TypedValue.COMPLEX_UNIT_SP, TOAST_SP);
        tv.setGravity(Gravity.CENTER);
        tv.setMaxLines(2);
        int padX = Math.round(TOAST_PAD_X_DP * density);
        int padY = Math.round(TOAST_PAD_Y_DP * density);
        tv.setPadding(padX, padY, padX, padY);

        android.graphics.drawable.GradientDrawable bg =
                new android.graphics.drawable.GradientDrawable();
        bg.setShape(android.graphics.drawable.GradientDrawable.RECTANGLE);
        bg.setCornerRadius(TOAST_RADIUS_DP * density);
        bg.setColor(0xF00E0E10);
        bg.setStroke(Math.max(1, Math.round(2f * density)), 0xFF4A9EFF);
        tv.setBackground(bg);
        return tv;
    }

    /**
     * The channel's importance AS THE PLATFORM HOLDS IT, not as this class asked
     * for it.
     *
     * <p>THE ONE FAILURE THAT LOOKS LIKE SUCCESS. {@code createNotificationChannel}
     * is a no-op when the channel already exists — the note on
     * {@link #ensureChannel} says so — and Android does not let an app RAISE an
     * importance the user has lowered. So a channel knocked down to
     * {@code IMPORTANCE_LOW} once, by the driver or by the ROM, can never raise
     * a heads-up banner again, while {@code notify} keeps returning normally and
     * this class kept reporting "posted". The station pop-up would be posted,
     * silent, and invisible, for the rest of the app's life on that unit.
     *
     * <p>The remedy is not code that fixes it — there is none — it is a NEW
     * channel id, which is a decision for a person. This exists so the person
     * can see they have to make it.
     *
     * <p>4 is HIGH and what this app asks for; 3 DEFAULT; 2 LOW; 1 MIN; 0 NONE.
     */
    private static String channelState() {
        try {
            NotificationManager nm = ctx.getSystemService(NotificationManager.class);
            if (nm == null) {
                return "channel unreadable";
            }
            NotificationChannel c = nm.getNotificationChannel(CHANNEL_ID);
            if (c == null) {
                return "channel missing";
            }
            int imp = c.getImportance();
            return imp >= NotificationManager.IMPORTANCE_HIGH
                    ? "channel importance " + imp
                    : "channel importance " + imp + " — TOO LOW FOR A BANNER";
        } catch (Throwable t) {
            return "channel unreadable: " + why(t);
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

    /** Take the pop-up down — the driver is back on the face and can see it. */
    public static synchronized void clear() {
        if (ctx == null) {
            return;
        }
        NotificationManager nm = ctx.getSystemService(NotificationManager.class);
        if (nm != null) {
            try {
                nm.cancel(NOTIFICATION_ID);
            } catch (Exception ignored) {
                // Cancelling something that is not there is not a fault.
            }
        }
    }

    /**
     * API 26+ refuses to post without a channel, and 26 is this app's floor, so
     * there is no branch — only the "already exists" case, which
     * {@code createNotificationChannel} treats as a no-op.
     *
     * <p>Made once per process rather than per post: the call is cheap but not
     * free, and this runs on every station change.
     */
    private static void ensureChannel(NotificationManager nm) {
        if (channelMade) {
            return;
        }
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID, "Station changed", NotificationManager.IMPORTANCE_HIGH);
        channel.setDescription("Says what is tuned when you change station from the wheel.");
        channel.setShowBadge(false);
        // NO SOUND AND NO VIBRATION. The banner is the message; a chime over the
        // radio the driver is listening to is not. IMPORTANCE_HIGH is what earns
        // the heads-up, and these two are what keep it quiet — the platform
        // otherwise gives a high channel the default notification tone.
        channel.setSound(null, null);
        channel.enableVibration(false);
        nm.createNotificationChannel(channel);
        channelMade = true;
    }

    /**
     * The station's logo, sized for a notification and not for a hero card.
     *
     * <p>TWO PASSES, WHICH IS NOT A FLOURISH. A master can be a thousand pixels
     * on a side and a large icon is drawn at about sixty-four dp; decoding the
     * former to show the latter allocates megabytes on a unit that has none to
     * spare. {@code inJustDecodeBounds} reads the header alone, and
     * {@code inSampleSize} then decodes at the nearest power-of-two step down.
     *
     * <p>Returns null for anything that does not decode — a missing file, a
     * format the platform will not read, a master that was truncated on the way
     * in. Nothing is lost when it does: the banner carries the call sign and the
     * dial regardless, and the mark was only ever beside them.
     */
    private static Bitmap decode(String path) {
        if (path == null || path.isEmpty()) {
            return null;
        }
        try {
            BitmapFactory.Options bounds = new BitmapFactory.Options();
            bounds.inJustDecodeBounds = true;
            BitmapFactory.decodeFile(path, bounds);
            int longest = Math.max(bounds.outWidth, bounds.outHeight);
            if (longest <= 0) {
                return null;
            }
            int want = Math.round(ICON_DP * ctx.getResources().getDisplayMetrics().density);
            int sample = 1;
            while (longest / (sample * 2) >= want) {
                sample *= 2;
            }
            BitmapFactory.Options opts = new BitmapFactory.Options();
            opts.inSampleSize = sample;
            return BitmapFactory.decodeFile(path, opts);
        } catch (Throwable t) {
            Log.i(TAG, "logo did not decode: " + t);
            return null;
        }
    }

    /**
     * Resolve the custom layout and its two text views, once.
     *
     * @return true when all three are present. A layout without its ids is a
     *     layout that has been edited out from under this code, and inflating it
     *     would give the driver an empty banner — so it is treated exactly like
     *     an absent one.
     */
    private static boolean ensureLayout() {
        if (layoutId == -1) {
            String pkg = ctx.getPackageName();
            layoutId = ctx.getResources().getIdentifier("station_popup", "layout", pkg);
            callId = ctx.getResources().getIdentifier("popup_call", "id", pkg);
            dialId = ctx.getResources().getIdentifier("popup_dial", "id", pkg);
            Log.i(TAG, layoutId == 0
                    ? "no custom layout in this build; using the platform template"
                    : "custom banner layout resolved");
        }
        return layoutId != 0 && callId != 0 && dialId != 0;
    }

    private static Notification build(String title, String text, Bitmap logo) {
        Notification.Builder b = new Notification.Builder(ctx, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.stat_sys_headset)
                // TRANSPORT, not SERVICE: this announces a change in what is
                // playing, which is what the category is for, and it is what
                // lets a head unit's own do-not-disturb rules treat it sanely.
                .setCategory(Notification.CATEGORY_TRANSPORT)
                // NOT ongoing and auto-cancelling: the driver can swipe it, and
                // tapping it takes them to the face.
                .setOngoing(false)
                .setAutoCancel(true)
                .setTimeoutAfter(TIMEOUT_MS)
                // FALSE, so every station change raises the banner again. The
                // default alerts once per id and then updates quietly, which for
                // one reused id would mean only the first change was ever seen.
                .setOnlyAlertOnce(false)
                .setShowWhen(false);
        // THE WORDS ALWAYS, THE MARK AS WELL WHEN THERE IS ONE. A logo-only
        // banner was tried and does not work: `setLargeIcon` draws into a small
        // square at the card's right edge, and station logos are landscape
        // wordmarks — the three in the handoff are 408x296, 545x200 and 255x144 —
        // so fitting one to that slot leaves the name a few pixels tall and the
        // rest of the card empty. The call sign and the dial are what a driver
        // can actually read at a glance; the mark identifies the station beside
        // them.
        b.setContentTitle(title == null || title.isEmpty() ? "Carnyx" : title);
        if (text != null && !text.isEmpty()) {
            b.setContentText(text);
        }

        boolean custom = ensureLayout();

        // THE LARGE ICON IS THE FALLBACK'S LOGO, AND ONLY THE FALLBACK'S.
        //
        // It was set unconditionally, which put the mark on screen TWICE in a
        // Gradle build: {@code DecoratedCustomViewStyle} keeps the platform's own
        // decoration, and the large icon is part of it — so the wordmark appeared
        // squeezed into the decoration's square slot AND again, correctly sized,
        // in the custom row below it. The squeezed one is exactly what the custom
        // layout was written to get rid of.
        //
        // Under cargo-apk there is no layout — that packager ships no resources —
        // and then the large icon IS the only way to show a logo at all, which is
        // why this is a branch rather than a deletion.
        if (logo != null && !custom) {
            b.setLargeIcon(logo);
        }

        // ── THE CUSTOM BANNER, WHEN THIS BUILD HAS ONE ────────────────────────
        //
        // Everything above still runs and is not wasted: it is what the platform
        // draws when the layout is absent, and on API 31+ the decorated template
        // reads the title and text for its own purposes even when a custom view
        // is set. This only replaces the BODY.
        //
        // DecoratedCustomViewStyle is the supported pairing — our view for the
        // content, the platform's for the header row. Setting a custom view
        // without it is the shape that has behaved differently on every release
        // since API 24.
        //
        // BOTH HOOKS, because they are different surfaces. `HeadsUp` is the
        // banner over another app, which is the whole feature; `Content` is the
        // entry left behind in the shade. Setting only the first leaves the two
        // looking like different notifications.
        if (custom) {
            RemoteViews rv = new RemoteViews(ctx.getPackageName(), layoutId);
            rv.setTextViewText(callId, title == null ? "" : title);
            rv.setTextViewText(dialId, text == null ? "" : text);
            int logoId = ctx.getResources().getIdentifier("popup_logo", "id", ctx.getPackageName());
            if (logoId != 0) {
                // GONE, not INVISIBLE, when there is nothing to show: the view has
                // to leave the measure or the words centre around a gap where a
                // logo would have been. The layout's own comment says the same.
                rv.setViewVisibility(logoId, logo == null ? View.GONE : View.VISIBLE);
                if (logo != null) {
                    rv.setImageViewBitmap(logoId, logo);
                }
            }
            b.setStyle(new Notification.DecoratedCustomViewStyle())
                    .setCustomContentView(rv)
                    .setCustomHeadsUpContentView(rv);
        }

        PendingIntent tap = launchIntent();
        if (tap != null) {
            b.setContentIntent(tap);
        }
        return b.build();
    }

    /**
     * Tapping the banner comes back to the face.
     *
     * <p>Built from the package manager's own launch intent rather than naming
     * {@code android.app.NativeActivity}: the activity's class is the
     * framework's, not ours. FLAG_IMMUTABLE is not optional — API 31+ rejects a
     * PendingIntent that declares neither mutability.
     */
    private static PendingIntent launchIntent() {
        Intent launch = ctx.getPackageManager().getLaunchIntentForPackage(ctx.getPackageName());
        if (launch == null) {
            return null;
        }
        return PendingIntent.getActivity(
                ctx, 0, launch, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }
}
