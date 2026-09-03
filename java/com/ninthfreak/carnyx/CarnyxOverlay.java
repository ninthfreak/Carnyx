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
    private static final float RADIUS_DP = 18f;

    /**
     * The frame around a mark, and around the words plate too.
     *
     * <p>Owner, on the shipped pop-up: <i>"they are all too small because the
     * logos aren't filling much of the full shape of the popup."</i> Two things
     * were eating it, and this is the smaller one. The toast's 28x18 margin
     * around a 162x101 plate made a 218x137 card of which the plate was 55% by
     * area before the art had been fitted into it at all. The words card had
     * the same margin and has it no longer: it is the mark card's size now,
     * and the same frame is what makes the two read as one card.
     */
    private static final float LOGO_PAD_DP = 10f;

    /**
     * The mark's box: how tall it is, and how wide it is allowed to get.
     *
     * <h2>Fitted to the art, not to a plate</h2>
     *
     * <p>This was the peek card's 162x101 box with the art {@code FIT_CENTER}
     * inside it, which is the larger half of the owner's complaint: a SQUARE
     * mark in a 16:10 box draws 101x101 and leaves 61dp of empty card either
     * side. Nothing is wrong with those numbers on a peek card, where the plate
     * is a fixed slot in a row of fixed slots; here there is no row, the card
     * wraps its content, and a box that does not match the art is dead space
     * with a border drawn round it.
     *
     * <p>So the box takes the ART'S aspect: {@link #LOGO_H_DP} tall, as wide as
     * that makes it, and if that would pass {@link #LOGO_MAX_W_DP} the height
     * gives way instead. A square mark is now 150x150 where it was 101x101 —
     * half again as large in each direction, and with no slack left inside it.
     *
     * <h2>Why 150 and not the peek card's 101</h2>
     *
     * <p>The peek plate is sized for Carnyx's own screen, which the driver has
     * opened and is looking at. This lands unannounced over a maps app and is
     * read in one glance — the same brief the words plate's type is sized to
     * meet ({@link #WORDS_CALL_FRAC}), and the same answer.
     */
    private static final float LOGO_H_DP = 150f;
    private static final float LOGO_MAX_W_DP = 300f;

    /**
     * A floor on the box's width, so the pop-up cannot become a hairline.
     *
     * <p>Art taller than about 1:1.6 would otherwise give a card narrower than
     * a third of its height, which reads as a strip rather than a card. Inside
     * this floor the mark letterboxes again — the one case where it does — and
     * no station wordmark is that shape, so it is insurance rather than a rule
     * anything is expected to hit.
     */
    private static final float LOGO_MIN_W_DP = 96f;

    /**
     * A plate's corner is 0.14 of its SHORT side — the peek card's rule, and
     * tighter than the card's own radius: the slab is a smaller rounded
     * rectangle sitting inside the card, not the card itself. Applied to the
     * mark's box and the words plate alike.
     *
     * <p>THE PEEK CARD'S 162x101 IS GONE FROM HERE. It was the first size of
     * both cards — asked for as <i>"roughly the same size as they are on the
     * prev/next peek cards"</i> — and the owner then found it too small over
     * another app. {@link #LOGO_H_DP} and {@link #WORDS_H_DP} are what replaced
     * it, and the reasoning is on each.
     */
    private static final float PLATE_RADIUS_FRAC = 0.14f;

    /**
     * The words plate, sized to the mark's box rather than the peek card's.
     *
     * <p>Owner, after the mark grew: <i>"Make the no-logo popup bigger to match
     * the logo one."</i> So the same {@link #LOGO_H_DP} tall, at the 16:10 the
     * call sign was fitted to — which is exactly the box a 16:10 mark gets —
     * and inside the same {@link #LOGO_PAD_DP} frame. A station with art and one
     * without now raise a card of the same size.
     *
     * <p>BOTH LINES GO INSIDE IT. The old card put the dial UNDER the plate, in
     * the card's own ink; that was the peek card's form before handoff v3.3.0
     * §13.1 moved the frequency into the plate, and the face's plates all hold
     * both lines now. Copying the current form is what "roughly the way they
     * are shown on peek cards" means today, and it is also the only way the
     * words card can be the mark card's size: a line beneath the plate would
     * make it taller than any mark.
     */
    private static final float WORDS_W_DP = 240f;
    private static final float WORDS_H_DP = 150f;

    /**
     * The plated logo's inset, as a fraction of the plate's long side.
     *
     * <p>0.09, which is `PlateBox`'s own figure, and it applies ONLY to
     * {@link #PLATE_GREY} art — a mark keyed to a grey slab is drawn expecting
     * that slab to show around it. Everything else fills the plate.
     */
    private static final float PLATED_PAD_FRAC = 0.09f;

    /**
     * The two lines inside the {@link #WORDS_H_DP} plate, as fractions of it.
     *
     * <p>NOT THE PEEK CARD'S TYPE, AND THAT IS DELIBERATE. A peek label is sized
     * for a card on Carnyx's own screen, which the driver has opened and is
     * looking straight at. This lands unannounced over a maps app and has to be
     * read in one glance before the eyes go back to the road — the brief the
     * 28sp toast was raised to meet. The first words card answered it with the
     * call sign at 34sp alone in the plate and the dial at 24sp under it; with
     * both lines in one plate the question is how the plate divides, and
     * handoff v3.3.0 §13.1's capture answers it. Measured off the WQLF plate there: the call sign's
     * ink band is 29% of the plate's height and the dial's 18%, the pair
     * centred with a small gap. Type size is the ink band over the cap height
     * — 0.72 for this face — so 0.29 / 0.72 = 0.40 and 0.18 / 0.72 = 0.25 of
     * the plate: 60 and 38 on a 150dp plate, against 34 and 24 before. The
     * call sign is the larger because it is the identity; the dial is the
     * fallback fact under it.
     * Two lines at those sizes are 118dp of line box in a 150dp plate, which
     * is the same air the face's plates carry.
     */
    private static final float WORDS_CALL_FRAC = 0.40f;
    private static final float WORDS_DIAL_FRAC = 0.25f;
    private static final float WORDS_GAP_FRAC = 0.02f;

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
            int brand, int plateInk, int ground, int ink, int edge, int logoFallback,
            int logoPlate, int plate) {
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
        // AT THE MARK'S OWN SIZE, not a 52dp strip and no longer the peek
        // plate's 162 either. The mark IS the pop-up when there is one, and
        // `card` now fits its box to the art up to LOGO_MAX_W_DP, so anything
        // decoded smaller than that is a mark being upscaled on screen.
        final Bitmap logo = CarnyxAlert.decodeLogo(logoPath, LOGO_MAX_W_DP);
        if (main == null) {
            main = new Handler(looper);
        }
        final Context app = ctx.getApplicationContext();
        boolean queued = main.post(new Runnable() {
            @Override public void run() {
                try {
                    removeNow(app);
                    View card = card(app, title, text, logo,
                            brand, plateInk, ground, ink, edge, logoFallback, logoPlate,
                            plate);
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
     * <h2>And the fallback is the peek card's own form, at the mark's size</h2>
     *
     * <p>No logo means the brand-filled plate with the call sign over the dial
     * inside it, which is what a peek card draws for a station with no art since
     * handoff v3.3.0 §13.1 — and it is drawn at exactly the box a 16:10 mark
     * would get, so the two kinds of pop-up are one size. See {@link #WORDS_W_DP}
     * for the size and {@link #WORDS_CALL_FRAC} for how the plate divides.
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
            int brand, int plateInk, int ground, int ink, int edge, int logoFallback,
            int logoPlate, int plate) {
        float density = ctx.getResources().getDisplayMetrics().density;

        LinearLayout col = new LinearLayout(ctx);
        col.setOrientation(LinearLayout.VERTICAL);
        col.setGravity(Gravity.CENTER_HORIZONTAL);

        GradientDrawable bg = new GradientDrawable();
        bg.setShape(GradientDrawable.RECTANGLE);
        bg.setCornerRadius(RADIUS_DP * density);
        bg.setColor(ground);
        bg.setStroke(Math.max(1, Math.round(2f * density)), edge);
        col.setBackground(bg);

        if (logo != null) {
            // THE BOX IS THE ART'S SHAPE. See LOGO_H_DP for why it is not the
            // peek plate's. A mark whose own bitmap reports nothing usable falls
            // back to a square, which is the shape that wastes least against an
            // unknown aspect.
            int artW = logo.getWidth();
            int artH = logo.getHeight();
            float aspect = (artW > 0 && artH > 0) ? ((float) artW / (float) artH) : 1f;
            float boxHdp = LOGO_H_DP;
            float boxWdp = boxHdp * aspect;
            if (boxWdp > LOGO_MAX_W_DP) {
                boxWdp = LOGO_MAX_W_DP;
                boxHdp = boxWdp / aspect;
            }
            if (boxWdp < LOGO_MIN_W_DP) {
                boxWdp = LOGO_MIN_W_DP;
            }
            int boxW = Math.max(1, Math.round(boxWdp * density));
            int boxH = Math.max(1, Math.round(boxHdp * density));
            int markRadius = Math.round(Math.min(boxW, boxH) * PLATE_RADIUS_FRAC);
            int markPad = Math.round(LOGO_PAD_DP * density);
            col.setPadding(markPad, markPad, markPad, markPad);

            int slab = plate == PLATE_FALLBACK ? logoFallback
                    : plate == PLATE_GREY ? logoPlate
                    : 0;
            ImageView mark = new ImageView(ctx);
            mark.setImageBitmap(logo);
            // CONTAIN, NEVER CROP (§4.5): the art scales to the largest size that
            // fits and is never cut, because half a wordmark is not a wordmark.
            // With the box now cut to the art's own aspect there is nothing left
            // for it to letterbox, which is the point.
            mark.setScaleType(ImageView.ScaleType.FIT_CENTER);
            if (slab != 0) {
                GradientDrawable under = new GradientDrawable();
                under.setShape(GradientDrawable.RECTANGLE);
                under.setCornerRadius(markRadius);
                under.setColor(slab);
                mark.setBackground(under);
            }
            if (plate == PLATE_GREY) {
                // Still `PlateBox`'s 0.09 of the long side, now measured on the
                // box this card actually draws. Grey-slab art is keyed to a slab
                // showing around it, so the inset is the slab, not padding.
                int inset = Math.round(Math.max(boxW, boxH) * PLATED_PAD_FRAC);
                mark.setPadding(inset, inset, inset, inset);
            }
            col.addView(mark, new LinearLayout.LayoutParams(boxW, boxH));
            return col;
        }

        // NO ART: the brand-coloured plate with BOTH lines in it — the call
        // sign over the dial, centred — at the mark card's size and inside the
        // mark card's frame. See WORDS_W_DP for why it is this shape and not the
        // peek card's, and WORDS_CALL_FRAC for how the plate divides. `Pal.flat`
        // has already desaturated the brand upstream if the face is dead, so
        // this spends the colour rather than deciding it.
        int wordsW = Math.round(WORDS_W_DP * density);
        int wordsH = Math.round(WORDS_H_DP * density);
        int wordsRadius = Math.round(Math.min(wordsW, wordsH) * PLATE_RADIUS_FRAC);
        int wordsPad = Math.round(LOGO_PAD_DP * density);
        col.setPadding(wordsPad, wordsPad, wordsPad, wordsPad);

        LinearLayout plateView = new LinearLayout(ctx);
        plateView.setOrientation(LinearLayout.VERTICAL);
        plateView.setGravity(Gravity.CENTER);
        GradientDrawable plateBg = new GradientDrawable();
        plateBg.setShape(GradientDrawable.RECTANGLE);
        plateBg.setCornerRadius(wordsRadius);
        plateBg.setColor(brand);
        plateView.setBackground(plateBg);

        // IN THE PLATE'S OWN INK, not white. `plateInk` is §5's measured answer
        // for this brand — the same `ink_on` the face's plates use — so WZEE and
        // WMHX read dark here exactly as they do on the rail.
        TextView callView = new TextView(ctx);
        callView.setText(call);
        callView.setTextColor(plateInk);
        // PX, NOT SP: the size is a share of a plate that is already in px.
        callView.setTextSize(TypedValue.COMPLEX_UNIT_PX, wordsH * WORDS_CALL_FRAC);
        callView.setGravity(Gravity.CENTER);
        callView.setMaxLines(1);
        callView.setIncludeFontPadding(false);
        plateView.addView(callView, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT));

        TextView dialView = new TextView(ctx);
        dialView.setText(dial);
        dialView.setTextColor(plateInk);
        dialView.setTextSize(TypedValue.COMPLEX_UNIT_PX, wordsH * WORDS_DIAL_FRAC);
        dialView.setGravity(Gravity.CENTER);
        dialView.setMaxLines(1);
        dialView.setIncludeFontPadding(false);
        LinearLayout.LayoutParams dialLp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT);
        dialLp.topMargin = Math.round(wordsH * WORDS_GAP_FRAC);
        plateView.addView(dialView, dialLp);

        col.addView(plateView, new LinearLayout.LayoutParams(wordsW, wordsH));
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
