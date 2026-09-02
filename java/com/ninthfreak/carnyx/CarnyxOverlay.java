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
    private static final float PAD_X_DP = 28f;
    private static final float PAD_Y_DP = 18f;
    private static final float RADIUS_DP = 18f;

    /**
     * THE PEEK CARD'S PLATE, and these are its numbers rather than new ones.
     *
     * <p>{@code ui/presets.slint} caps a peek plate at 184 and scales the card by
     * 0.88, giving 162 wide; the box is aspect-locked 16:10, so 101 tall. Asked
     * for directly — <i>"roughly the same size as they are on the prev/next peek
     * cards"</i> — and taken from the source rather than eyeballed, so the two
     * stay the same size if either moves.
     *
     * <p>The plate's own corner is 0.14 of its SHORT side, which is the peek's
     * rule too and is tighter than the card's radius: the slab is a smaller
     * rounded rectangle sitting inside the card, not the card itself.
     */
    private static final float PLATE_W_DP = 162f;
    private static final float PLATE_H_DP = 101f;
    private static final float PLATE_RADIUS_FRAC = 0.14f;

    /**
     * The plated logo's inset, as a fraction of the plate's long side.
     *
     * <p>0.09, which is `PlateBox`'s own figure, and it applies ONLY to
     * {@link #PLATE_GREY} art — a mark keyed to a grey slab is drawn expecting
     * that slab to show around it. Everything else fills the plate.
     */
    private static final float PLATED_PAD_FRAC = 0.09f;

    /**
     * Type for the no-logo card, in sp.
     *
     * <p>NOT THE PEEK CARD'S 14, AND THAT IS DELIBERATE. The peek label is sized
     * for a card on Carnyx's own screen, which the driver has opened and is
     * looking straight at. This lands unannounced over a maps app and has to be
     * read in one glance before the eyes go back to the road, which is the same
     * brief the 28sp toast was raised to meet. So the peek's LAYOUT is copied and
     * its type size is not: the call sign at 34, the dial under it at 24.
     *
     * <p>The call sign is the larger of the two because it is the identity; the
     * dial is the fallback fact beside it. On the peek card the same pair is a
     * box and a label, and this is that relationship at a readable size.
     */
    private static final float CALL_SP = 34f;
    private static final float DIAL_SP = 24f;

    /** {@code LogoPlate}, as `app::plate_code` numbers it. */
    private static final int PLATE_LIGHT = 0;
    private static final int PLATE_FALLBACK = 1;
    private static final int PLATE_BARE = 2;
    private static final int PLATE_GREY = 3;

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
    static String show(Context ctx, String title, String text, String logoPath,
            int brand, int ground, int ink, int edge, int logoFallback, int logoPlate,
            int plate) {
        if (ctx == null) {
            return "no overlay: no context";
        }
        if (!permitted(ctx)) {
            return "no overlay: not permitted";
        }
        if (title == null || title.isEmpty()) {
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
        // AT THE PLATE'S SIZE NOW, not a 52dp strip. The mark IS the pop-up when
        // there is one, so it is decoded for a 162x101 box.
        final Bitmap logo = CarnyxAlert.decodeLogo(logoPath, PLATE_W_DP);
        if (main == null) {
            main = new Handler(looper);
        }
        final Context app = ctx.getApplicationContext();
        boolean queued = main.post(new Runnable() {
            @Override public void run() {
                try {
                    removeNow(app);
                    View card = card(app, title, text, logo,
                            brand, ground, ink, edge, logoFallback, logoPlate, plate);
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

    /**
     * The card: the mark alone when there is one, the call sign and dial when
     * there is not.
     *
     * <h2>The logo stands by itself</h2>
     *
     * <p>Asked for directly — <i>"show the station logo … and not bother with the
     * call sign or frequency as text"</i> — and it is right here even though
     * {@link CarnyxAlert#build}'s note says a logo-only banner was tried and
     * failed. THAT NOTE IS ABOUT THE NOTIFICATION and does not carry over: there
     * the mark went through {@code setLargeIcon}, which draws into a small square
     * at the card's right edge, and a landscape wordmark in a square slot is
     * unreadable. Here the plate is a 16:10 box this class sizes itself, which is
     * the shape the art was made for. An earlier version of this file repeated
     * the notification's conclusion as if it applied; it did not.
     *
     * <h2>And the fallback is the peek card's own form</h2>
     *
     * <p>No logo means the brand-filled box with the call sign in it and the dial
     * beneath, which is exactly what a peek card draws for a station with no art.
     * The layout is copied; the type size is not — see {@link #CALL_SP}.
     *
     * <h2>What goes behind the art</h2>
     *
     * <p>The four-state {@code LogoPlate} decision, made in Rust and spent here.
     * A mark adapted for a dark face needs a KNOWN ground rather than whatever
     * the card happens to be: {@link #PLATE_FALLBACK} is un-adapted light art
     * that needs white paper under it, {@link #PLATE_GREY} is keyed to a grey
     * slab and insets itself so the slab shows, and {@link #PLATE_BARE} and
     * {@link #PLATE_LIGHT} need nothing. Getting this wrong is a logo that
     * vanishes into the card, which is why it travels rather than being guessed
     * from the card colour.
     */
    private static View card(Context ctx, String call, String dial, Bitmap logo,
            int brand, int ground, int ink, int edge, int logoFallback, int logoPlate,
            int plate) {
        float density = ctx.getResources().getDisplayMetrics().density;
        int padX = Math.round(PAD_X_DP * density);
        int padY = Math.round(PAD_Y_DP * density);
        int plateW = Math.round(PLATE_W_DP * density);
        int plateH = Math.round(PLATE_H_DP * density);
        int plateRadius = Math.round(Math.min(plateW, plateH) * PLATE_RADIUS_FRAC);

        LinearLayout col = new LinearLayout(ctx);
        col.setOrientation(LinearLayout.VERTICAL);
        col.setGravity(Gravity.CENTER_HORIZONTAL);
        col.setPadding(padX, padY, padX, padY);

        GradientDrawable bg = new GradientDrawable();
        bg.setShape(GradientDrawable.RECTANGLE);
        bg.setCornerRadius(RADIUS_DP * density);
        bg.setColor(ground);
        bg.setStroke(Math.max(1, Math.round(2f * density)), edge);
        col.setBackground(bg);

        if (logo != null) {
            int slab = plate == PLATE_FALLBACK ? logoFallback
                    : plate == PLATE_GREY ? logoPlate
                    : 0;
            ImageView mark = new ImageView(ctx);
            mark.setImageBitmap(logo);
            // CONTAIN, NEVER CROP (§4.5): the art scales to the largest size that
            // fits and is never cut, because half a wordmark is not a wordmark.
            mark.setScaleType(ImageView.ScaleType.FIT_CENTER);
            if (slab != 0) {
                GradientDrawable under = new GradientDrawable();
                under.setShape(GradientDrawable.RECTANGLE);
                under.setCornerRadius(plateRadius);
                under.setColor(slab);
                mark.setBackground(under);
            }
            if (plate == PLATE_GREY) {
                int inset = Math.round(Math.max(plateW, plateH) * PLATED_PAD_FRAC);
                mark.setPadding(inset, inset, inset, inset);
            }
            col.addView(mark, new LinearLayout.LayoutParams(plateW, plateH));
            return col;
        }

        // NO ART: the brand-coloured box with the call letters in it, then the
        // dial. `Pal.flat` has already desaturated the brand upstream if the face
        // is dead, so this spends the colour rather than deciding it.
        TextView box = new TextView(ctx);
        box.setText(call);
        box.setTextColor(0xFFFFFFFF);
        box.setTextSize(TypedValue.COMPLEX_UNIT_SP, CALL_SP);
        box.setGravity(Gravity.CENTER);
        box.setMaxLines(1);
        GradientDrawable plateBg = new GradientDrawable();
        plateBg.setShape(GradientDrawable.RECTANGLE);
        plateBg.setCornerRadius(plateRadius);
        plateBg.setColor(brand);
        box.setBackground(plateBg);
        col.addView(box, new LinearLayout.LayoutParams(plateW, plateH));

        TextView under = new TextView(ctx);
        under.setText(dial);
        under.setTextColor(ink);
        under.setTextSize(TypedValue.COMPLEX_UNIT_SP, DIAL_SP);
        under.setGravity(Gravity.CENTER);
        under.setMaxLines(1);
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT);
        // The peek card's own gap: a third of its label's size.
        lp.topMargin = Math.round(DIAL_SP * 0.34f * density);
        col.addView(under, lp);
        return col;
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
