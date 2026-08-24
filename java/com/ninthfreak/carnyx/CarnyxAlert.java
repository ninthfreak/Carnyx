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
import android.util.Log;

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

    private static Context ctx;
    private static boolean channelMade;

    private CarnyxAlert() {
    }

    /** Hand the class the app context, as {@link CarnyxProcess#attach} does. */
    public static synchronized void attach(Context context) {
        if (ctx == null && context != null) {
            ctx = context.getApplicationContext();
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
     * @return true when the platform was handed the notification. False means it
     *     could not be posted AND the reason is in the log; it is never a silent
     *     no.
     */
    public static synchronized boolean post(String title, String text, String logoPath) {
        if (ctx == null) {
            Log.i(TAG, "post() before attach()");
            return false;
        }
        NotificationManager nm = ctx.getSystemService(NotificationManager.class);
        if (nm == null) {
            Log.w(TAG, "no NotificationManager");
            return false;
        }
        // API 33+ only. Below it the method exists and answers for the app as a
        // whole, which is still worth honouring: a driver who turned Carnyx's
        // notifications off in Settings has said what they want.
        if (!nm.areNotificationsEnabled()) {
            Log.i(TAG, "notifications are off for this app; nothing posted");
            return false;
        }
        ensureChannel(nm);
        try {
            nm.notify(NOTIFICATION_ID, build(title, text, decode(logoPath)));
            return true;
        } catch (Exception e) {
            Log.w(TAG, "notify failed: " + e);
            return false;
        }
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
        if (logo != null) {
            b.setLargeIcon(logo);
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
