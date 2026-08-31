package com.ninthfreak.carnyx;

import android.app.AppOpsManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.ResolveInfo;
import android.content.pm.ServiceInfo;
import android.content.pm.Signature;
import android.os.Build;
import android.provider.Settings;
import android.util.Log;

import java.io.File;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;

/**
 * Where the stock radio app could be intercepted, WITHOUT ROOT.
 *
 * <h2>The question</h2>
 *
 * <p>The MCU remembers the current source across a sleep and restores it on
 * ACC-on, so a unit left on FM comes back into FM and the stock radio app
 * launches itself over whatever Carnyx was doing. The idea under investigation
 * is a TRAMPOLINE: something that inherits the firmware's own restore decision
 * and turns it into a launch of Carnyx, instead of Carnyx reimplementing that
 * decision (which is what the wake receiver, #95, does).
 *
 * <h2>The plan this probe is NOT for</h2>
 *
 * <p>CarFM had a trampoline plan and shipped a recon probe for it
 * (`VibeStreamModule.probeTrampolineFeasibility`). ITS PLAN IS UNAVAILABLE HERE
 * and this is not that probe carried across. That plan was to move the vendor
 * APK off {@code /system} and install a same-named stub in its place, and every
 * step of it needs root: remounting {@code /system} rw, deleting the system
 * copy, then installing a package whose name is already taken. The owner has no
 * root on this unit and no easy route to it, so the whole branch is closed and
 * nothing here reads a verity property, stats a partition, looks for a
 * {@code su} binary, or shells out at all. CarFM's probe ran {@code su -c id}
 * deliberately, to raise a Superuser prompt as its answer; this one runs no
 * process whatsoever.
 *
 * <h2>What is left without root, and what each one needs</h2>
 *
 * <p>Four routes survive. The report is organised around their preconditions,
 * because each is cheap to read and each can kill its route outright.
 *
 * <ol>
 *   <li>NO TRAMPOLINE AT ALL — {@code pm disable-user --user 0 <pkg>} from an
 *       adb shell. Needs no root, and if it works there is nothing to build. The
 *       report reads whether adb and the developer settings are even enabled,
 *       because that is the difference between "one command" and "a project".
 *   <li>BECOME A HANDLER — only possible if the firmware launches the app by an
 *       IMPLICIT intent. Then Carnyx can declare the same filter and be picked.
 *       The action sweep below is what says whether such a door exists.
 *   <li>JUMP IN FRONT — notice the stock app take the foreground and come back
 *       over it. Needs an exemption from the Android 10 background-activity-start
 *       restriction, and the two that a driver can grant in Settings are a
 *       notification listener and an accessibility service. Read, not assumed.
 *   <li>SHARE ITS TASK — an activity declaring the stock app's own
 *       {@code taskAffinity}. The affinities are reported for exactly this, and
 *       it is listed last because it is the most fragile.
 * </ol>
 *
 * <h2>What it reads, and what it will not do</h2>
 *
 * <p>READ-ONLY, NO ROOT, NO SHELL, NO PROCESS. Every call below is a public
 * PackageManager query, a world-readable {@code Settings.Secure} row, or
 * {@code File.canRead()} on a path the package manager handed out. Nothing is
 * launched, enabled, disabled or written. The one thing this probe recommends
 * that it does not do is copying the stock APK off the unit — a file manager
 * already can, and reading its manifest properly is a desktop job.
 *
 * <h2>Two limits worth knowing before reading the output</h2>
 *
 * <p>PACKAGE VISIBILITY, as {@link CarnyxKeepAlive} says: unfiltered on this
 * unit (Android 10), nearly empty from API 30 where the manifest's
 * {@code <queries>} covers the tuner service alone. On a newer unit most of this
 * report would be missing rather than negative.
 *
 * <p>INTENT FILTERS ARE NOT READABLE FROM OUTSIDE A PACKAGE. The platform will
 * answer "who handles THIS intent" but will not enumerate what a package
 * declares, so the sweep below can only ask about actions this code already
 * knows to name. A silent action — a vendor string nobody has seen — is
 * invisible to it, and the honest way to that answer is the APK on disk.
 */
public final class CarnyxStockRadio {
    private static final String TAG = "CarnyxStockRadio";

    /** The vendor's tuner service. NOT a candidate: it must survive untouched. */
    private static final String SERVICE_PKG = "com.nwd.radio.service";

    /** What CarFM recorded as the stock app, checked rather than assumed. */
    private static final String EXPECTED_PKG = "com.nwd.radio";

    /**
     * Actions to ask "who answers this?".
     *
     * <p>The vendor half is every {@code com.nwd.*} string this tree has ever
     * seen, from `NwdBridge` and CarFM's radio module. The framework half is the
     * shapes a head unit plausibly uses to bring a radio forward.
     *
     * <p>A MISS PROVES NOTHING, which is the whole reason the note above about
     * unreadable intent filters exists. A HIT is decisive: it names a door.
     */
    private static final String[] ACTIONS = {
        "com.nwd.ACTION_OS_WAKE_UP",
        // BOTH ACC-OFF SPELLINGS, because this sweep is now the only thing that
        // can say whether `SleepReceiver` registered for each. It is declared for
        // the two in the manifest — the vendor writes this action unqualified
        // where it writes the others in full, and nobody knows which the ROM
        // sends — and a drive log showed only the unqualified one resolving,
        // which proved nothing about the other because only the other was never
        // asked about.
        "com.nwd.ACTION_ACCOFF_UPDATE",
        "com.nwd.action.ACTION_ACCOFF_UPDATE",
        "com.nwd.ACTION_ILL_STATE_CHANGE",
        "com.nwd.action.ACTION_KEY_VALUE",
        "com.nwd.action.ACTION_REQUEST_CHANGE_SOURCE",
        "com.nwd.action.ACTION_APP_IN_OUT",
        "com.nwd.action.ACTION_TEST_KEY",
        "com.nwd.radio.service.ACTION_RADIO_SERVICE",
        "android.intent.action.MEDIA_BUTTON",
        "android.media.action.MEDIA_PLAY_FROM_SEARCH",
        "android.media.browse.MediaBrowserService",
        "android.intent.action.MUSIC_PLAYER",
    };

    /** Categories to pair with {@code ACTION_MAIN}, which alone resolves nothing. */
    private static final String[] MAIN_CATEGORIES = {
        Intent.CATEGORY_LAUNCHER,
        Intent.CATEGORY_HOME,
        Intent.CATEGORY_CAR_DOCK,
        Intent.CATEGORY_APP_MUSIC,
    };

    /** The ring holds 600 lines and this is one of several writers. */
    private static final int MAX_ACTIVITIES = 18;
    private static final int MAX_RECEIVERS = 12;
    private static final int MAX_SERVICES = 8;
    private static final int MAX_CANDIDATES = 12;
    private static final int MAX_HANDLERS_PER_ACTION = 4;

    private static Context ctx;

    private CarnyxStockRadio() {
    }

    /** Hand the class the app context, as {@link CarnyxProcess#attach} does. */
    public static synchronized void attach(Context context) {
        if (ctx == null && context != null) {
            ctx = context.getApplicationContext();
        }
    }

    /**
     * The report, newline-separated. Never throws and never returns null — a
     * probe that crashes the app it is diagnosing is worse than no probe.
     */
    public static synchronized String report() {
        List<String> out = new ArrayList<>();
        if (ctx == null) {
            return "stock radio probe: no context";
        }
        out.add("stock radio probe: android " + Build.VERSION.SDK_INT
                + ", " + Build.MANUFACTURER + " " + Build.MODEL);
        String pkg = candidates(out);
        if (pkg != null) {
            app(out, pkg);
            components(out, pkg);
        }
        actions(out, pkg);
        home(out);
        foreground(out);
        shellRoute(out);
        limits(out, pkg);
        return join(out);
    }

    // ── 1. Which package ─────────────────────────────────────────────────────

    /**
     * Find the stock radio app rather than trusting a constant.
     *
     * <p>CarFM recorded {@code com.nwd.radio} with a launcher activity of
     * {@code com.nwd.radio.home_horizontalActivity}, and that is a note about
     * ONE unit's firmware from another project's session. It is checked here and
     * the alternatives are listed, because a ROM update that renamed the app
     * would otherwise make this whole report quietly describe nothing.
     *
     * @return the package to detail, or null if none is visible
     */
    private static String candidates(List<String> out) {
        String best = null;
        int shown = 0;
        try {
            PackageManager pm = ctx.getPackageManager();
            String self = ctx.getPackageName();
            for (PackageInfo p : pm.getInstalledPackages(0)) {
                if (p == null || p.packageName == null) {
                    continue;
                }
                String n = p.packageName;
                if (n.equals(self) || n.equals(SERVICE_PKG)) {
                    continue;
                }
                String l = n.toLowerCase(Locale.US);
                if (!(l.startsWith("com.nwd") || l.contains("radio") || l.contains(".fm"))) {
                    continue;
                }
                boolean launchable = pm.getLaunchIntentForPackage(n) != null;
                if (shown < MAX_CANDIDATES) {
                    out.add("  candidate " + n + (launchable ? " (launchable)" : ""));
                    shown++;
                }
                // Prefer what CarFM recorded; otherwise the first one a driver
                // could actually see on screen.
                if (n.equals(EXPECTED_PKG)) {
                    best = n;
                } else if (best == null && launchable) {
                    best = n;
                }
            }
        } catch (Throwable t) {
            out.add("  candidates: unreadable (" + t.getClass().getSimpleName() + ")");
        }
        if (best == null) {
            out.add("  NO STOCK RADIO APP VISIBLE — which on API 30+ may be filtering, not absence");
        } else {
            out.add("  detailing " + best + (best.equals(EXPECTED_PKG)
                    ? " (matches what CarFM recorded)"
                    : " (NOT com.nwd.radio — CarFM's note is stale or this is another unit)"));
        }
        return best;
    }

    // ── 2. The app itself ────────────────────────────────────────────────────

    /**
     * Identity, install shape, and the two facts that decide the no-root routes.
     *
     * <p>THE APK PATH AND ITS READABILITY are the most valuable lines here.
     * Intent filters cannot be enumerated through the package manager (see the
     * class note), and the APK is where they are actually written down — a
     * {@code /system} APK is world-readable, so a file manager can copy it to a
     * USB stick and the question can be answered properly on a desktop instead
     * of guessed at from a car.
     *
     * <p>THE SIGNER COMPARISON is the one thing that could revive the
     * same-name plan without root: if the vendor app and Carnyx shared a signing
     * certificate, installing a package with its name would be an ordinary
     * upgrade and no {@code /system} surgery would be involved. It will almost
     * certainly say no. It is measured because "almost certainly" is not an
     * answer and the check costs one call.
     */
    private static void app(List<String> out, String pkg) {
        try {
            PackageManager pm = ctx.getPackageManager();
            ApplicationInfo ai = pm.getApplicationInfo(pkg, 0);
            PackageInfo pi = pm.getPackageInfo(pkg, 0);
            boolean sys = (ai.flags & ApplicationInfo.FLAG_SYSTEM) != 0;
            boolean upd = (ai.flags & ApplicationInfo.FLAG_UPDATED_SYSTEM_APP) != 0;
            out.add("  app version " + pi.versionName + ", uid " + ai.uid
                    + (sys ? ", system" : ", NOT system") + (upd ? ", has /data overlay" : ""));
            String src = ai.sourceDir;
            if (src == null) {
                out.add("  apk: no sourceDir");
            } else {
                File f = new File(src);
                boolean readable = f.canRead();
                out.add("  apk " + src);
                out.add("  apk " + (readable ? "READABLE" : "not readable")
                        + ", " + (f.length() / 1024) + " KB"
                        + (readable ? " — copy it off and read its manifest properly" : ""));
            }
            out.add("  enabled setting " + pm.getApplicationEnabledSetting(pkg)
                    + " (0 default, 1 enabled, 2 disabled, 3 disabled-user)");
            Intent launch = pm.getLaunchIntentForPackage(pkg);
            out.add("  launches as " + (launch == null || launch.getComponent() == null
                    ? "(no launcher activity)"
                    : launch.getComponent().flattenToShortString()));
            boolean shared = shareSigner(pm, pkg);
            out.add("  same signer as Carnyx: " + shared
                    + (shared
                        ? " — a same-name install would be an ORDINARY UPGRADE, no root needed"
                        : " — a same-name install is impossible without removing the system copy"));
        } catch (Throwable t) {
            out.add("  app: unreadable (" + t.getClass().getSimpleName() + ")");
        }
    }

    /** Do the two packages share a signing certificate? See {@link #app}. */
    private static boolean shareSigner(PackageManager pm, String pkg) {
        Set<String> ours = digests(pm, ctx.getPackageName());
        Set<String> theirs = digests(pm, pkg);
        if (ours.isEmpty() || theirs.isEmpty()) {
            return false;
        }
        for (String d : ours) {
            if (theirs.contains(d)) {
                return true;
            }
        }
        return false;
    }

    /**
     * SHA-256 of each signing certificate.
     *
     * <p>The API-28 split is not cosmetic: {@code GET_SIGNATURES} reports the
     * CURRENT signer only and was deprecated for exactly the rotation case
     * {@code signingCertificateHistory} exists to cover. minSdk here is 26, so
     * both branches are reachable in principle even though this unit is 29.
     */
    @SuppressWarnings("deprecation")
    private static Set<String> digests(PackageManager pm, String pkg) {
        Set<String> out = new LinkedHashSet<>();
        try {
            Signature[] sigs;
            if (Build.VERSION.SDK_INT >= 28) {
                PackageInfo pi = pm.getPackageInfo(pkg, PackageManager.GET_SIGNING_CERTIFICATES);
                if (pi.signingInfo == null) {
                    return out;
                }
                sigs = pi.signingInfo.hasMultipleSigners()
                        ? pi.signingInfo.getApkContentsSigners()
                        : pi.signingInfo.getSigningCertificateHistory();
            } else {
                sigs = pm.getPackageInfo(pkg, PackageManager.GET_SIGNATURES).signatures;
            }
            if (sigs == null) {
                return out;
            }
            for (Signature s : sigs) {
                byte[] d = MessageDigest.getInstance("SHA-256").digest(s.toByteArray());
                StringBuilder hex = new StringBuilder();
                for (byte b : d) {
                    hex.append(String.format(Locale.US, "%02X", b));
                }
                out.add(hex.toString());
            }
        } catch (Throwable t) {
            Log.w(TAG, "digests(" + pkg + "): " + t);
        }
        return out;
    }

    // ── 3. Its components ────────────────────────────────────────────────────

    /**
     * What the stock app is made of, and the three attributes that matter to a
     * trampoline.
     *
     * <p>{@code exported} says whether a component can be addressed from
     * outside at all. {@code launchMode} and {@code taskAffinity} are what route
     * (4) in the class note turns on — an activity of ours declaring the same
     * affinity lands in the same task, which is the only no-root way to be
     * WHERE the stock app is rather than merely after it.
     *
     * <p>Names only, no filters: see the class note. This is the shape of the
     * app, not its doors.
     */
    private static void components(List<String> out, String pkg) {
        try {
            PackageManager pm = ctx.getPackageManager();
            PackageInfo pi = pm.getPackageInfo(pkg,
                    PackageManager.GET_ACTIVITIES
                            | PackageManager.GET_RECEIVERS
                            | PackageManager.GET_SERVICES);
            String defaultAffinity = pkg;
            int n = 0;
            if (pi.activities != null) {
                out.add("  activities: " + pi.activities.length);
                for (ActivityInfo a : pi.activities) {
                    if (n++ >= MAX_ACTIVITIES) {
                        break;
                    }
                    String affinity = a.taskAffinity == null ? "(none)" : a.taskAffinity;
                    out.add("    act " + shortName(a.name, pkg)
                            + (a.exported ? " exported" : " internal")
                            + " mode=" + a.launchMode
                            + (affinity.equals(defaultAffinity) ? "" : " affinity=" + affinity)
                            + (a.permission == null ? "" : " perm=" + a.permission));
                }
                if (pi.activities.length > MAX_ACTIVITIES) {
                    out.add("    … " + (pi.activities.length - MAX_ACTIVITIES) + " more not shown");
                }
            }
            n = 0;
            if (pi.receivers != null) {
                out.add("  receivers: " + pi.receivers.length);
                for (ActivityInfo r : pi.receivers) {
                    if (n++ >= MAX_RECEIVERS) {
                        break;
                    }
                    out.add("    rcv " + shortName(r.name, pkg)
                            + (r.exported ? " exported" : " internal"));
                }
                if (pi.receivers.length > MAX_RECEIVERS) {
                    out.add("    … " + (pi.receivers.length - MAX_RECEIVERS) + " more not shown");
                }
            }
            n = 0;
            if (pi.services != null) {
                out.add("  services: " + pi.services.length);
                for (ServiceInfo s : pi.services) {
                    if (n++ >= MAX_SERVICES) {
                        break;
                    }
                    out.add("    svc " + shortName(s.name, pkg)
                            + (s.exported ? " exported" : " internal"));
                }
                if (pi.services.length > MAX_SERVICES) {
                    out.add("    … " + (pi.services.length - MAX_SERVICES) + " more not shown");
                }
            }
        } catch (Throwable t) {
            out.add("  components: unreadable (" + t.getClass().getSimpleName() + ")");
        }
    }

    /** Drop the package prefix, which is on every line and reads as noise. */
    private static String shortName(String name, String pkg) {
        if (name == null) {
            return "(null)";
        }
        return name.startsWith(pkg + ".") ? name.substring(pkg.length()) : name;
    }

    // ── 4. The doors ─────────────────────────────────────────────────────────

    /**
     * Who answers each action this code knows to name — the sweep that would
     * find an implicit door if one exists.
     *
     * <p>A HIT ON THE STOCK APP IS THE FINDING. It means the firmware could be
     * launching the app through an action rather than a component, and an action
     * is something Carnyx can declare too — which turns the whole problem into a
     * chooser and a default. Everything else in this report is context for that
     * one line.
     *
     * <p>Activities, receivers and services are asked separately because they
     * are three different registries and a broadcast door is as good as an
     * activity door for this purpose.
     */
    private static void actions(List<String> out, String pkg) {
        PackageManager pm = ctx.getPackageManager();
        int doors = 0;
        for (String action : ACTIONS) {
            doors += sweep(out, pm, new Intent(action), action, pkg);
        }
        for (String cat : MAIN_CATEGORIES) {
            Intent i = new Intent(Intent.ACTION_MAIN).addCategory(cat);
            doors += sweep(out, pm, i, "MAIN+" + shortCategory(cat), pkg);
        }
        if (pkg != null && doors == 0) {
            out.add("  NO IMPLICIT DOOR FOUND on any action named above");
            out.add("  → which is not proof there is none; unnamed actions are invisible here");
        }
    }

    /**
     * One intent, three registries.
     *
     * @return 1 if the stock app answered, 0 otherwise
     */
    private static int sweep(List<String> out, PackageManager pm, Intent intent,
            String label, String pkg) {
        List<String> hits = new ArrayList<>();
        boolean stock = false;
        try {
            for (ResolveInfo r : pm.queryIntentActivities(intent, 0)) {
                if (r.activityInfo == null) {
                    continue;
                }
                stock |= add(hits, r.activityInfo.packageName, "act", pkg);
            }
            for (ResolveInfo r : pm.queryBroadcastReceivers(intent, 0)) {
                if (r.activityInfo == null) {
                    continue;
                }
                stock |= add(hits, r.activityInfo.packageName, "rcv", pkg);
            }
            for (ResolveInfo r : pm.queryIntentServices(intent, 0)) {
                if (r.serviceInfo == null) {
                    continue;
                }
                stock |= add(hits, r.serviceInfo.packageName, "svc", pkg);
            }
        } catch (Throwable t) {
            out.add("  " + label + ": unreadable (" + t.getClass().getSimpleName() + ")");
            return 0;
        }
        if (hits.isEmpty()) {
            // Silent. A no-handler action is the common case and one line each
            // would be twenty lines of nothing in a 600-line ring.
            return 0;
        }
        String joined = join(hits, ", ");
        if (hits.size() > MAX_HANDLERS_PER_ACTION) {
            joined = join(hits.subList(0, MAX_HANDLERS_PER_ACTION), ", ")
                    + " +" + (hits.size() - MAX_HANDLERS_PER_ACTION);
        }
        out.add((stock ? "  DOOR " : "  ") + label + " → " + joined);
        return stock ? 1 : 0;
    }

    /** @return true if this handler is the stock app */
    private static boolean add(List<String> hits, String handler, String kind, String pkg) {
        if (handler == null) {
            return false;
        }
        boolean stock = handler.equals(pkg);
        String entry = kind + ":" + handler + (stock ? "*" : "");
        if (!hits.contains(entry)) {
            hits.add(entry);
        }
        return stock;
    }

    private static String shortCategory(String cat) {
        int dot = cat.lastIndexOf('.');
        return dot < 0 ? cat : cat.substring(dot + 1);
    }

    // ── 5. Who owns the restore ──────────────────────────────────────────────

    /**
     * The HOME apps, because whoever answers HOME is the likeliest owner of the
     * decision that brings the radio forward.
     *
     * <p>This probe cannot see that decision. Naming the package says which APK
     * to pull and read next, which is the same advice the stock APK's own line
     * carries.
     */
    private static void home(List<String> out) {
        try {
            PackageManager pm = ctx.getPackageManager();
            Intent i = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME);
            List<ResolveInfo> res = pm.queryIntentActivities(i, 0);
            if (res.isEmpty()) {
                out.add("  home: none visible");
                return;
            }
            for (ResolveInfo r : res) {
                if (r.activityInfo != null) {
                    out.add("  home " + r.activityInfo.packageName + "/"
                            + shortName(r.activityInfo.name, r.activityInfo.packageName));
                }
            }
        } catch (Throwable t) {
            out.add("  home: unreadable (" + t.getClass().getSimpleName() + ")");
        }
    }

    // ── 6. Can we take the foreground from behind ────────────────────────────

    /**
     * The three user-grantable permissions that decide whether route (3) —
     * notice the stock app and come back over it — is possible at all.
     *
     * <p>THE RULE THIS MEASURES. From Android 10 an app in the background cannot
     * start an activity, with a documented list of exemptions. Two of them are
     * reachable without root and without a developer: an app with a service the
     * SYSTEM binds is exempt, and both {@code NotificationListenerService} and
     * an {@code AccessibilityService} are system-bound. A driver grants either
     * one from Settings, and either would also cover the wake receiver's known
     * risk (#95).
     *
     * <p>Usage access is the third and is not an exemption — it is how the stock
     * app's arrival would be NOTICED in the first place. All three are read from
     * the platform's own record rather than inferred: an app cannot tell whether
     * it holds a special access by trying and seeing nothing happen.
     *
     * <p>NOTHING IS REQUESTED HERE. Reading which of these are held is the
     * question; prompting for them is a decision the owner has not made.
     */
    private static void foreground(List<String> out) {
        String self = ctx.getPackageName();
        out.add("  " + grant("notification listener", "enabled_notification_listeners", self));
        out.add("  " + grant("accessibility", "enabled_accessibility_services", self));
        out.add("  " + usageAccess(self));
        out.add("  → either of the first two exempts Carnyx from the Android 10");
        out.add("    background-activity-start block; the third is how it would notice");
    }

    /**
     * Is our package named in one of the {@code Settings.Secure} grant lists?
     *
     * <p>These are colon-separated lists of flattened component names, so the
     * test is for our package followed by a separator — a bare
     * {@code contains(pkg)} would match a package whose name merely starts with
     * ours.
     */
    private static String grant(String label, String key, String self) {
        try {
            String v = Settings.Secure.getString(ctx.getContentResolver(), key);
            if (v == null || v.isEmpty()) {
                return label + ": nothing enabled on this unit";
            }
            for (String entry : v.split(":")) {
                String e = entry.trim();
                if (e.equals(self) || e.startsWith(self + "/")) {
                    return label + ": GRANTED to Carnyx";
                }
            }
            return label + ": not Carnyx (" + v.split(":").length + " other(s) enabled)";
        } catch (Throwable t) {
            return label + ": unreadable (" + t.getClass().getSimpleName() + ")";
        }
    }

    /**
     * Usage access, through AppOps rather than a permission check.
     *
     * <p>{@code PACKAGE_USAGE_STATS} is an appop-backed permission: holding it
     * in the manifest means nothing, and only the op says whether a driver has
     * turned it on in Settings. The API-29 rename is a rename only —
     * {@code unsafeCheckOpNoThrow} is the same call under a name that admits it
     * does not verify the caller.
     */
    @SuppressWarnings("deprecation")
    private static String usageAccess(String self) {
        try {
            AppOpsManager ops = ctx.getSystemService(AppOpsManager.class);
            if (ops == null) {
                return "usage access: no AppOpsManager";
            }
            int uid = ctx.getApplicationInfo().uid;
            int mode = Build.VERSION.SDK_INT >= 29
                    ? ops.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, uid, self)
                    : ops.checkOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, uid, self);
            return "usage access: " + (mode == AppOpsManager.MODE_ALLOWED
                    ? "GRANTED to Carnyx" : "not granted (mode " + mode + ")");
        } catch (Throwable t) {
            return "usage access: unreadable (" + t.getClass().getSimpleName() + ")";
        }
    }

    // ── 7. The route that needs no code ──────────────────────────────────────

    /**
     * Whether an adb shell is available, because it beats everything above.
     *
     * <p>{@code pm disable-user --user 0 <pkg>} needs NO ROOT — the shell user
     * holds the permission it wants — and it stops the stock app from being
     * launched at all, by anything. If that works there is no trampoline to
     * build, and CarFM's own note already named the cheap experiment: disable it
     * and run one ignition cycle.
     *
     * <p>These two rows say whether that route is open on this unit or whether
     * someone has to find the developer settings first. They are read, never
     * written: turning adb on for a driver is not a diagnostic's decision.
     */
    private static void shellRoute(List<String> out) {
        out.add("  " + global("adb", Settings.Global.ADB_ENABLED));
        out.add("  " + global("developer settings",
                Settings.Global.DEVELOPMENT_SETTINGS_ENABLED));
        out.add("  → with adb: `pm disable-user --user 0 <pkg>` needs no root and");
        out.add("    would end this whole question; one ignition cycle proves it");
    }

    private static String global(String label, String key) {
        try {
            String v = Settings.Global.getString(ctx.getContentResolver(), key);
            if (v == null) {
                return label + ": unset";
            }
            return label + ": " + ("1".equals(v.trim()) ? "ENABLED" : "off (" + v + ")");
        } catch (Throwable t) {
            return label + ": unreadable (" + t.getClass().getSimpleName() + ")";
        }
    }

    // ── 8. What this cannot answer ───────────────────────────────────────────

    /**
     * The questions no amount of reading settles, stated so the report is not
     * mistaken for a verdict.
     *
     * <p>CarFM's probe ended the same way and it was the right instinct: the
     * expensive mistake is treating a recon dump as a decision.
     */
    private static void limits(List<String> out, String pkg) {
        String name = pkg == null ? "<stock pkg>" : pkg;
        out.add("  CANNOT ANSWER, in the order they matter:");
        out.add("   1. by package or by component? only disabling " + name
                + " + one ACC cycle says");
        out.add("   2. what the app's real intent filters are — read the APK off the unit");
        out.add("   3. whether the tuner service needs the app; it must stay either way");
        out.add("   4. whether a background start is actually permitted, until one is tried");
    }

    // ── Plumbing ─────────────────────────────────────────────────────────────

    private static String join(List<String> lines) {
        String text = join(lines, "\n");
        Log.i(TAG, "report: " + lines.size() + " lines");
        return text;
    }

    private static String join(List<String> parts, String sep) {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < parts.size(); i++) {
            if (i > 0) {
                b.append(sep);
            }
            b.append(parts.get(i));
        }
        return b.toString();
    }
}
