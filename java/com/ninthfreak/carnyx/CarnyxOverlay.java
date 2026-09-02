package com.ninthfreak.carnyx;

import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.PixelFormat;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.util.Log;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.WindowManager;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;

/**
 * The station pop-up as a window this app owns, drawn over whatever is in front.
 *
 * <h2>Why a third rendering of the same thing</h2>
 *
 * <p>{@link CarnyxAlert} already posts a notification and raises a toast, and
 * both have a ceiling this does not:
 *
 * <ul>
 *   <li>THE NOTIFICATION IS INVISIBLE ON THIS ROM. A drive produced two clean
 *       {@code posted, channel importance 4} lines and the owner saw no banner
 *       and no shade entry — SystemUI on a head unit of this class often has
 *       neither. And from API 33 an ungranted {@code POST_NOTIFICATIONS} makes
 *       the platform drop it outright.
 *   <li>THE CUSTOM TOAST STOPS BEING ALLOWED AT API 30, which deprecated
 *       {@code setView}, blocked custom toasts from the background and made
 *       {@code setGravity} a no-op for text toasts. The large, top-placed card
 *       degrades to a small grey box at the bottom, and a logo could not ride on
 *       it at all.
 * </ul>
 *
 * <p>A {@code TYPE_APPLICATION_OVERLAY} window has neither ceiling. It needs no
 * channel, no shade and no SystemUI cooperation, nothing about it is deprecated,
 * and it is the only one of the three that can carry an image on every Android
 * this app supports. It is tried FIRST and the toast becomes the fallback.
 *
 * <h2>The cost, which is a permission no dialog can ask for</h2>
 *
 * <p>{@code SYSTEM_ALERT_WINDOW} is a "special" permission: from API 23 no
 * runtime dialog can request it and the driver switches it on at Settings'
 * "Display over other apps" screen. {@link #requestPermission} sends them there.
 * On a ROM with a stripped Settings app that screen may not exist at all — the
 * keep-alive probe's {@code overlay:} lines are what say whether it does, and
 * this class degrades to "not permitted" rather than assuming.
 *
 * <h2>IT MUST NEVER EAT A TOUCH</h2>
 *
 * <p>{@code FLAG_NOT_TOUCHABLE} and {@code FLAG_NOT_FOCUSABLE} together, and
 * they are the most important two lines in this file. This window appears
 * unbidden over a maps app while the car is moving; a pop-up that swallowed a
 * tap meant for the road ahead would be a hazard, not a feature. Every touch
 * passes through to whatever is underneath, which also means the pop-up cannot
 * be dismissed by tapping it — hence {@link #SHOW_MS}, and {@link #hide} when
 * the driver returns to the face.
 *
 * <h2>Where it lives</h2>
 *
 * <p>{@code java/}, the runtime dex, beside {@link CarnyxAlert}, so BOTH
 * packagers get it — {@link CarnyxService} is in the Gradle source set and does
 * not exist under cargo-apk, and this needs nothing that a plain {@code Context}
 * cannot provide. Rust never calls this class directly: {@link CarnyxAlert} is
 * the single entry point for the pop-up and delegates here, which keeps the JNI
 * seam at one class.
 */
final class CarnyxOverlay {
    private static final String TAG = "CarnyxOverlay";

    /**
     * How long the window stays up.
     *
     * <p>Longer than a toast's {@code LENGTH_LONG} (about 3.5s) because this one
     * cannot be dismissed by tapping it — see the note on touchability — and
     * because it exists for a driver who looked away from the road and needs to
     * find the answer when they look back. Short enough that it is gone before
     * it becomes something to wait out.
     */
    private static final long SHOW_MS = 5000L;

    /** Matching {@link CarnyxAlert}'s toast, so the two read as one design. */
    private static final float TEXT_SP = 28f;
    private static final float PAD_X_DP = 28f;
    private static final float PAD_Y_DP = 18f;
    private static final float RADIUS_DP = 18f;

    /**
     * The logo's height in the card, in dp.
     *
     * <p>Larger than the notification's {@code ICON_DP} of 64 and sized against
     * the TEXT rather than against a system slot: a notification's large icon is
     * a fixed square the platform chooses, and this is a mark sitting beside
     * 28sp words on a card this class draws itself. Height-constrained with the
     * width left to follow, because station logos are landscape — the
     * notification's own note records a logo-only banner failing for exactly the
     * reason a square crop would.
     */
    private static final float LOGO_DP = 52f;

    /** The live window, or null. Touched only on the main thread. */
    private static View shown;

    private static Handler main;

    private CarnyxOverlay() {
    }

    /**
     * Is the permission held right now?
     *
     * <p>Cheap enough to ask on every station change: it is an app-op lookup and
     * not a package-manager walk. Asked every time rather than cached because a
     * driver can grant or revoke it in Settings while the app is running, and a
     * cached "no" would outlive the grant that fixed it.
     */
    static boolean permitted(Context ctx) {
        try {
            return Settings.canDrawOverlays(ctx);
        } catch (Throwable t) {
            return false;
        }
    }

    /**
     * Send the driver to the grant screen.
     *
     * @return one line for the diagnostics log, never null.
     */
    static String requestPermission(Context ctx, android.app.Activity activity) {
        if (ctx == null) {
            return "overlay permission: no context";
        }
        if (permitted(ctx)) {
            return "overlay permission: already granted";
        }
        try {
            Intent i = new Intent("android.settings.action.MANAGE_OVERLAY_PERMISSION",
                    Uri.parse("package:" + ctx.getPackageName()));
            // THE ACTIVITY WHEN THERE IS ONE, the app context otherwise. Starting
            // an Activity from a non-Activity context needs NEW_TASK, and on
            // API 29+ a background start would be refused outright — but this is
            // only ever reached from a settings row, with the face in front, so
            // the Activity is normally there and the fallback is for the case
            // where it has been collected.
            if (activity != null) {
                activity.startActivity(i);
            } else {
                i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                ctx.startActivity(i);
            }
            return "overlay permission: sent you to Android's \"Display over other"
                    + " apps\" screen — switch Carnyx on there";
        } catch (Throwable t) {
            // A ROM with no such screen throws ActivityNotFoundException here,
            // and that is a real answer rather than a bug: it means the route is
            // closed on this unit. The keep-alive probe reports the same thing
            // ahead of time.
            return "overlay permission: this ROM has no \"Display over other apps\""
                    + " screen — " + t.getClass().getSimpleName();
        }
    }

    /**
     * Put the card up, replacing one already showing.
     *
     * @return one clause for the diagnostics log, never null.
     */
    static String show(Context ctx, String title, String text, String logoPath) {
        if (ctx == null) {
            return "no overlay: no context";
        }
        if (!permitted(ctx)) {
            return "no overlay: not permitted";
        }
        final String message = message(title, text);
        if (message.isEmpty()) {
            return "no overlay: nothing to say";
        }
        Looper looper = Looper.getMainLooper();
        if (looper == null) {
            return "no overlay: no main looper";
        }
        // THE BITMAP IS DECODED HERE, off the main thread. This is called from
        // the poll's thread and decoding is the one expensive step in the whole
        // path; doing it inside the posted Runnable would put file I/O on the UI
        // thread of an app that is drawing a radio face.
        final Bitmap logo = CarnyxAlert.decodeLogo(logoPath, LOGO_DP);
        if (main == null) {
            main = new Handler(looper);
        }
        final Context app = ctx.getApplicationContext();
        boolean queued = main.post(new Runnable() {
            @Override public void run() {
                try {
                    removeNow(app);
                    View card = card(app, message, logo);
                    WindowManager wm = app.getSystemService(WindowManager.class);
                    if (wm == null) {
                        return;
                    }
                    wm.addView(card, params(app));
                    shown = card;
                    main.postDelayed(new Runnable() {
                        @Override public void run() {
                            removeNow(app);
                        }
                    }, SHOW_MS);
                } catch (Throwable t) {
                    // The likely one is the permission being revoked between the
                    // check above and this call. Nothing to recover and nobody to
                    // tell: `show` has already returned its line to the log.
                    Log.w(TAG, "overlay failed: " + t);
                }
            }
        });
        return queued ? "overlay shown" : "no overlay: the main looper refused it";
    }

    /** Take it down early — the driver is back on the face. */
    static void hide(Context ctx) {
        if (ctx == null || main == null) {
            return;
        }
        final Context app = ctx.getApplicationContext();
        main.post(new Runnable() {
            @Override public void run() {
                removeNow(app);
            }
        });
    }

    /** Main thread only. Safe to call with nothing showing. */
    private static void removeNow(Context app) {
        View v = shown;
        shown = null;
        if (v == null) {
            return;
        }
        try {
            WindowManager wm = app.getSystemService(WindowManager.class);
            if (wm != null) {
                wm.removeView(v);
            }
        } catch (Throwable t) {
            // Already gone, which happens when the window was torn down with the
            // Activity. Not worth a line.
            Log.i(TAG, "overlay already removed: " + t);
        }
    }

    /** The same one line the toast shows, so the two never disagree. */
    private static String message(String title, String text) {
        if (title == null || title.isEmpty()) {
            return text == null ? "" : text;
        }
        if (text == null || text.isEmpty()) {
            return title;
        }
        return title + "  ·  " + text;
    }

    /**
     * The card: the mark on the left when there is one, the words beside it.
     *
     * <p>BESIDE AND NOT ABOVE, and not alone. {@link CarnyxAlert#build}'s note
     * records that a logo-only banner was tried and does not work — the driver
     * needs the call sign and the dial, and the mark is what makes them findable
     * at a glance rather than what replaces them.
     */
    private static View card(Context ctx, String message, Bitmap logo) {
        float density = ctx.getResources().getDisplayMetrics().density;
        int padX = Math.round(PAD_X_DP * density);
        int padY = Math.round(PAD_Y_DP * density);

        LinearLayout row = new LinearLayout(ctx);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.setPadding(padX, padY, padX, padY);

        GradientDrawable bg = new GradientDrawable();
        bg.setShape(GradientDrawable.RECTANGLE);
        bg.setCornerRadius(RADIUS_DP * density);
        bg.setColor(0xF00E0E10);
        bg.setStroke(Math.max(1, Math.round(2f * density)), 0xFF4A9EFF);
        row.setBackground(bg);

        if (logo != null) {
            ImageView mark = new ImageView(ctx);
            mark.setImageBitmap(logo);
            // FIT_CENTER WITH A FIXED HEIGHT AND A FREE WIDTH. Station logos are
            // landscape and vary widely in ratio; constraining both axes would
            // either letterbox them into a square or stretch them, and the whole
            // value of a mark is that it is recognised without being read.
            mark.setScaleType(ImageView.ScaleType.FIT_CENTER);
            mark.setAdjustViewBounds(true);
            int h = Math.round(LOGO_DP * density);
            LinearLayout.LayoutParams lp =
                    new LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, h);
            lp.rightMargin = Math.round(16f * density);
            row.addView(mark, lp);
        }

        TextView tv = new TextView(ctx);
        tv.setText(message);
        tv.setTextColor(0xFFFFFFFF);
        tv.setTextSize(TypedValue.COMPLEX_UNIT_SP, TEXT_SP);
        tv.setGravity(Gravity.CENTER_VERTICAL);
        tv.setMaxLines(2);
        row.addView(tv);
        return row;
    }

    /**
     * Where the window sits and what it refuses to do.
     *
     * <p>{@code TYPE_APPLICATION_OVERLAY} unconditionally: it arrived at API 26
     * and {@code minSdk} is 26 in both packagers, so the deprecated
     * {@code TYPE_PHONE} branch every guide still carries would be dead code
     * here.
     *
     * <p>The two flags are the safety property — see the class note. Top-centred
     * one sixteenth down, matching the toast exactly, so replacing one with the
     * other does not move the pop-up.
     */
    private static WindowManager.LayoutParams params(Context ctx) {
        WindowManager.LayoutParams lp = new WindowManager.LayoutParams(
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
                        | WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE,
                PixelFormat.TRANSLUCENT);
        lp.gravity = Gravity.TOP | Gravity.CENTER_HORIZONTAL;
        lp.y = ctx.getResources().getDisplayMetrics().heightPixels / 16;
        return lp;
    }
}
