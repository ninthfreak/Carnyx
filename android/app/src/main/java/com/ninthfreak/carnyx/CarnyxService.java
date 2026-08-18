package com.ninthfreak.carnyx;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

/**
 * The foreground service, and it exists for ONE reason: to keep this process
 * alive while the driver is in another app.
 *
 * <p>The fault it answers, in the owner's words: "When switching to a different
 * app and then switching back, it looked like the app was starting fresh, having
 * to draw elements and wait for things like radio text to be decoded." CarFM
 * never had that, because {@code VibeStreamService} runs in the FOREGROUND even
 * when the built-in NWD tuner is the source and the service carries no audio at
 * all (VibeStreamService.kt:726-739). A process with a foreground service is not
 * a candidate for the launcher's cleaner or the low-memory killer, so switching
 * away and back is a resume with every byte of state still in RAM.
 *
 * <h2>Why this file is in android/ and not in java/</h2>
 *
 * Carnyx has two Java trees and they are not interchangeable. {@code java/} is
 * compiled by {@code build.rs}, dexed, embedded with {@code include_bytes!} and
 * loaded at RUN time by an {@code InMemoryDexClassLoader} — that is how a pure
 * NativeActivity gets a tuner binder at all. This file cannot live there: Android
 * instantiates a manifest-declared component through the APPLICATION's own class
 * loader, which knows nothing about a class loader Rust built after start-up, so
 * a service class in the runtime dex would be a {@code ClassNotFoundException}
 * every time the system tried to construct it.
 *
 * <p>So it lives in the Gradle source set, where AGP compiles it into the APK's
 * own {@code classes.dex}. That is the whole reason #67 was blocked on the
 * packager: cargo-apk packages no Java and has no {@code <service>} field, so
 * under cargo-apk this class does not exist and cannot be made to.
 * {@link CarnyxProcess} is written to expect exactly that and to log it rather
 * than throw.
 *
 * <h2>The notification is a tax, not a feature</h2>
 *
 * Android requires a foreground service to post one. It is deliberately
 * IMPORTANCE_LOW: no sound, no vibration, no heads-up — an ongoing "this is
 * running" line, which is what it honestly is.
 *
 * <p>ON API 33+ THE DRIVER MAY NEVER SEE IT, AND THAT IS FINE. Posting needs the
 * POST_NOTIFICATIONS runtime permission, and this app never asks for it — the
 * manifest's own note about location applies here word for word: a runtime
 * request "needs someone to tap Allow, which on a dashboard at night is nobody".
 * When the permission is absent the platform suppresses the notification but
 * STILL RUNS THE SERVICE IN THE FOREGROUND, and the foreground is the entire
 * point. The permission is declared so a driver who wants the line can grant it
 * in Settings; nothing here depends on their doing so.
 */
public final class CarnyxService extends Service {
    private static final String TAG = "CarnyxService";

    /** Stable across restarts; the channel is created once and reused. */
    private static final String CHANNEL_ID = "carnyx.running";
    private static final int NOTIFICATION_ID = 1;

    /** Optional text for the notification's second line — see CarnyxProcess. */
    public static final String EXTRA_TEXT = "com.ninthfreak.carnyx.extra.TEXT";

    @Override
    public IBinder onBind(Intent intent) {
        // Nothing binds to this. It is started and stopped, and its only job is
        // to exist.
        return null;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        // A null intent is what a restarted service is handed; the text is
        // optional either way, so there is nothing to fail over.
        String text = intent == null ? null : intent.getStringExtra(EXTRA_TEXT);
        try {
            startForeground(text);
        } catch (Exception e) {
            // Android 12+ throws ForegroundServiceStartNotAllowedException when
            // the start came from the background, and API 34 throws when the
            // declared type and the granted permission disagree. Either way the
            // process simply does not get pinned; nothing else in the app
            // depends on this call, so the honest response is a log line and a
            // stop rather than taking the face down with it.
            Log.w(TAG, "could not enter the foreground: " + e);
            stopSelf();
            return START_NOT_STICKY;
        }
        // NOT sticky, deliberately. Sticky would have Android resurrect this
        // service after the process died — a notification, and a live process,
        // with no face behind either and no state left to protect. The service
        // is meant to prevent that death, not to outlive it.
        return START_NOT_STICKY;
    }

    /**
     * Post the notification and enter the foreground, with the API-34 type
     * handling the platform now insists on.
     */
    private void startForeground(String text) {
        ensureChannel();

        Notification notification = buildNotification(text);
        if (Build.VERSION.SDK_INT >= 29) {
            // API 29 added the typed form and API 34 REQUIRES it: a service
            // declaring a foregroundServiceType in the manifest must pass the
            // matching constant here, and the app must hold the matching
            // FOREGROUND_SERVICE_* permission. mediaPlayback, because claiming
            // the FM source is what makes the unit's audio play — the bytes go
            // through the MCU rather than through this process, but the
            // playback happens because this app asked for it.
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
        } else {
            startForeground(NOTIFICATION_ID, notification);
        }
    }

    /**
     * API 26+ refuses to post without a channel, and 26 is this app's floor
     * (skia-bindings hardcodes it), so there is no branch here — only the
     * "already exists" case, which createNotificationChannel treats as a no-op.
     */
    private void ensureChannel() {
        NotificationManager nm = getSystemService(NotificationManager.class);
        if (nm == null) {
            return;
        }
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID, "Radio running", NotificationManager.IMPORTANCE_LOW);
        channel.setDescription("Keeps Carnyx in memory while you are in another app.");
        channel.setShowBadge(false);
        nm.createNotificationChannel(channel);
    }

    private Notification buildNotification(String text) {
        Notification.Builder b = new Notification.Builder(this, CHANNEL_ID)
                .setContentTitle("Carnyx")
                .setSmallIcon(android.R.drawable.stat_sys_headset)
                .setOngoing(true)
                // NO setPriority. It has been deprecated since API 26 — the
                // CHANNEL's importance governs from there on, and 26 is this
                // app's floor, so IMPORTANCE_LOW above is the whole statement.
                // Setting both would be one of them doing nothing.
                .setCategory(Notification.CATEGORY_SERVICE)
                .setShowWhen(false);
        if (text != null && !text.isEmpty()) {
            b.setContentText(text);
        }
        PendingIntent tap = launchIntent();
        if (tap != null) {
            b.setContentIntent(tap);
        }
        return b.build();
    }

    /**
     * Tapping the line comes back to the face.
     *
     * <p>Built from the package manager's own launch intent rather than naming
     * {@code android.app.NativeActivity} here: the activity's class is the
     * framework's, not ours, and asking the package manager keeps this correct
     * if the manifest's launcher entry ever changes.
     *
     * <p>FLAG_IMMUTABLE is not optional — API 31+ rejects a PendingIntent that
     * declares neither mutability.
     */
    private PendingIntent launchIntent() {
        Intent launch = getPackageManager().getLaunchIntentForPackage(getPackageName());
        if (launch == null) {
            return null;
        }
        return PendingIntent.getActivity(
                this, 0, launch, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    @Override
    public void onDestroy() {
        // Nothing to release: no binder, no wake lock, no thread. The
        // notification goes with the service.
        Log.i(TAG, "stopped");
        super.onDestroy();
    }
}
