package com.ninthfreak.carnyx;

import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.ResolveInfo;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.PowerManager;
import android.os.SystemClock;
import android.provider.Settings;
import android.util.Log;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/**
 * What, if anything, could keep this app alive while the head unit sleeps.
 *
 * <h2>The question</h2>
 *
 * <p>The MCU sleeps the SoC on ACC-off and kills apps around it. CarFM recorded
 * that as settled — "the process is killed, so it cannot observe the sleep" — and
 * built its whole wake design on coming back rather than surviving. Nothing in
 * either project has ever checked WHY it is killed, or whether this ROM has the
 * keep-alive list that vendor Androids usually do.
 *
 * <p>This is not CarFM's probe carried across. CarFM has no such thing; its
 * vendor probes asked about the tuner and the boot behaviour, and they have been
 * removed from this tree. This one is new, it asks Carnyx's own question, and it
 * exists because the answer decides whether the wake receiver (#95) is the whole
 * story or only half of it.
 *
 * <h2>What it reads, and what it will not do</h2>
 *
 * <p>READ-ONLY, NO ROOT, NO SHELL. Everything below is a public API or a
 * world-readable provider. The one probe CarFM had with a real blast radius
 * shelled out and could prompt for root; this deliberately does not, because a
 * question about power management is not worth a chance of changing it.
 *
 * <ul>
 *   <li>WHETHER THE UNIT REBOOTS OR MERELY SUSPENDS across an ignition cycle,
 *       from a boot marker taken at every launch. The whole tree has assumed the
 *       answer — inherited from CarFM — and it decides whether
 *       {@code BOOT_COMPLETED} is in play at all. See {@link #bootMarker}.
 *   <li>WHICH THIRD-PARTY PACKAGES ARE FORCE-STOPPED right now, which is the one
 *       reading that separates an ordinary memory kill from the vendor cleaner
 *       the owner's app-switcher report points at. See {@link #stoppedApps}.
 *   <li>The accessibility and notification-listener grants — the two routes the
 *       platform itself keeps alive and re-binds, rather than the app registering
 *       something and hoping. See {@link #survivable}.
 *   <li>Who DECLARES a receiver for each wake and sleep action this app has ever
 *       tried, which says whether the vendor action strings are even real on this
 *       ROM. See {@link #autoStart}.
 *   <li>The battery-optimisation exemption, which governs Doze rather than a
 *       vendor sleep — worth recording so it can be RULED OUT rather than
 *       assumed.
 *   <li>Rows of {@code Settings.Global}, {@code .Secure} and {@code .System}
 *       whose NAME reads like a keep-alive list. Vendor Androids very often keep
 *       one there under a name of their own invention, which is exactly why the
 *       match is on a keyword rather than on a key this code guesses.
 *   <li>Packages that plausibly own such a list, so the owner knows where to look
 *       in the unit's own settings.
 * </ul>
 *
 * <h2>WHEN TO TAP THE ROW</h2>
 *
 * <p>ON THE FIRST LAUNCH AFTER AN IGNITION CYCLE, and the report is much weaker
 * at any other moment. {@link #stoppedApps} can only see a force-stopped
 * neighbour before the driver has opened it, and {@link #bootMarker} describes
 * the gap between the last two launches, which is the interesting gap only while
 * the most recent one is the wake. Tapping it twice in one sitting answers a
 * question about the last two taps and nothing about the sleep.
 *
 * <h2>Two limits worth knowing before reading the output</h2>
 *
 * <p>PACKAGE VISIBILITY. From API 30 an app sees only packages it has declared,
 * so the list is complete on THIS unit (Android 10) and would be nearly empty on
 * a newer one. The manifest's {@code <queries>} covers the tuner service alone.
 *
 * <p>AN EMPTY RESULT IS NOT AN ANSWER. A ROM can hold its list in a private
 * provider, a file, or its own service, none of which is reachable from here.
 * Nothing found means nothing found, which the report says in those words rather
 * than reporting "no whitelist".
 */
public final class CarnyxKeepAlive {
    private static final String TAG = "CarnyxKeepAlive";

    /**
     * What a keep-alive setting tends to be called. Matched against the setting's
     * NAME, lower-cased, as a substring.
     *
     * <p>Deliberately broad. The cost of a false positive is one line in a log
     * the owner is already reading; the cost of a false negative is not finding
     * the thing this exists to find.
     */
    private static final String[] KEYWORDS = {
        "whitelist", "white_list", "protect", "autostart", "auto_start", "auto_run",
        "selfstart", "self_start", "startup", "keepalive", "keep_alive", "freeze",
        "hibernat", "standby", "doze", "idle", "restrict", "persist",
        "background", "bg_", "powersave", "power_save", "clean", "kill",
        "accessib", "listener", "recent", "acc", "sleep", "wake", "boot",
    };

    /** The ring holds 600 lines and this is one of several writers. */
    private static final int MAX_SETTINGS = 40;
    private static final int MAX_PACKAGES = 25;
    private static final int MAX_STOPPED = 20;
    private static final int MAX_HANDLERS = 8;

    /**
     * {@code ApplicationInfo.FLAG_STOPPED}, which is {@code @hide}.
     *
     * <p>THE CONSTANT IS HIDDEN; THE BIT IS NOT. {@link ApplicationInfo#flags} is
     * a public field and reading it is an ordinary field read — no reflection, so
     * none of the API-28+ hidden-member restrictions apply. Only the NAME of this
     * bit is unavailable to third-party code, so the name is written here.
     *
     * <p>Worth the trouble because it is the one thing that separates "Android
     * killed the process to reclaim memory" from "something force-stopped the
     * package". They look identical from the driver's seat and they need
     * completely different fixes: a killed app still receives broadcasts and is
     * still in the app switcher, a force-stopped one receives NOTHING — not the
     * vendor's wake action, not {@code BOOT_COMPLETED}, which is otherwise exempt
     * from every delivery restriction there is — until a human taps its icon.
     */
    private static final int FLAG_STOPPED = 0x00200000;

    /** Where the boot marker lives. Private to this class; see {@link #attach}. */
    private static final String PREFS = "carnyx_probe";
    private static final String KEY_BOOT_AT = "boot_at";
    private static final String KEY_ELAPSED = "elapsed";
    private static final String KEY_SEEN_AT = "seen_at";

    private static Context ctx;

    /**
     * The boot marker as it stood BEFORE this launch overwrote it, or null on the
     * first run since install. See {@link #attach} and {@link #bootMarker}.
     */
    private static long[] previous;

    private CarnyxKeepAlive() {
    }

    /**
     * Hand the class the app context, as {@link CarnyxProcess#attach} does, and
     * take the boot marker while the previous one is still on disk.
     *
     * <p>THE MARKER IS TAKEN HERE AND NOT IN {@link #report}, because this runs
     * on every launch ({@code probe::init}, from {@code android_main}) and the
     * report runs only when a driver taps the row. A marker written at tap time
     * would compare this tap against the previous TAP — two points that can be
     * weeks and a hundred ignition cycles apart — and answer a question nobody
     * asked. Read-then-write, with the old pair kept in {@link #previous}, so the
     * report describes the gap between the last two LAUNCHES.
     */
    public static synchronized void attach(Context context) {
        if (ctx != null || context == null) {
            return;
        }
        ctx = context.getApplicationContext();
        try {
            SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            long elapsed = SystemClock.elapsedRealtime();
            long now = System.currentTimeMillis();
            if (p.contains(KEY_BOOT_AT)) {
                previous = new long[] {
                    p.getLong(KEY_BOOT_AT, 0L),
                    p.getLong(KEY_ELAPSED, 0L),
                    p.getLong(KEY_SEEN_AT, 0L),
                };
            }
            // `apply`, not `commit`: this sits on the start-up path and nothing
            // reads the value back until the NEXT launch, so there is nothing to
            // race and no reason to block the first frame on a disk write. The
            // opposite of the receivers' choice, for the opposite reason — see
            // WakeReceiver.note.
            p.edit()
                    .putLong(KEY_BOOT_AT, now - elapsed)
                    .putLong(KEY_ELAPSED, elapsed)
                    .putLong(KEY_SEEN_AT, now)
                    .apply();
        } catch (Throwable t) {
            Log.w(TAG, "could not take the boot marker: " + t);
        }
    }

    /**
     * The report, newline-separated. Never throws and never returns null — a
     * probe that crashes the app it is diagnosing is worse than no probe.
     */
    public static synchronized String report() {
        List<String> out = new ArrayList<>();
        if (ctx == null) {
            return "keep-alive probe: no context";
        }
        out.add("keep-alive probe: android " + Build.VERSION.SDK_INT
                + ", " + Build.MANUFACTURER + " " + Build.MODEL);
        bootMarker(out);
        stoppedApps(out);
        survivable(out);
        autoStart(out);
        battery(out);
        settings(out);
        packages(out);
        return join(out);
    }

    /**
     * Did the OS REBOOT between the last two launches, or did the SoC merely
     * suspend and resume?
     *
     * <p>THIS IS THE QUESTION EVERY OTHER ANSWER HANGS OFF, and until now the
     * whole tree has assumed it. {@code TASKS.md} states flatly that the unit does
     * not cold-boot on an ignition cycle, and that assumption was INHERITED from
     * CarFM rather than measured here. It decides whether {@code BOOT_COMPLETED}
     * is even in play: it is the one broadcast exempt from the API-26 ban on
     * manifest receivers hearing implicit intents, so if the unit really does boot
     * then the wake receiver's fallback path is live and the vendor action is a
     * distraction.
     *
     * <p>{@code elapsedRealtime} COUNTS DEEP SLEEP AND RESETS ONLY ON BOOT, which
     * is exactly the property wanted — {@code uptimeMillis} stops during suspend
     * and would call every long sleep a reboot. A value LOWER than the one stored
     * at the previous launch is unforgeable proof of a reboot, so that is the
     * verdict this reports as certain.
     *
     * <p>The boot instant ({@code now - elapsedRealtime}) is reported beside it as
     * the corroborating signal, and is NOT treated as proof: a head unit sets its
     * clock from GPS or the network some seconds after coming up, and that jump
     * moves the computed boot instant without anything having rebooted. Two
     * signals that can disagree, with the reason they disagree written down, beats
     * one signal presented as certainty.
     */
    private static void bootMarker(List<String> out) {
        long elapsed = SystemClock.elapsedRealtime();
        out.add("  uptime: " + describe(elapsed) + " since boot");
        if (previous == null) {
            out.add("  boot marker: first launch since install — nothing to compare yet."
                    + " Run this again after one ignition cycle.");
            return;
        }
        long prevBootAt = previous[0];
        long prevElapsed = previous[1];
        long prevSeenAt = previous[2];
        out.add("  last launch: " + describe(System.currentTimeMillis() - prevSeenAt) + " ago");
        if (elapsed < prevElapsed) {
            out.add("  boot marker: THE OS REBOOTED between the last two launches"
                    + " (uptime went backwards, " + describe(prevElapsed)
                    + " -> " + describe(elapsed) + "). BOOT_COMPLETED is in play.");
            return;
        }
        long drift = Math.abs((System.currentTimeMillis() - elapsed) - prevBootAt);
        if (drift > 60_000L) {
            out.add("  boot marker: uptime rose but the boot instant moved by "
                    + describe(drift) + " — probably a reboot the uptime could not"
                    + " prove, possibly just the clock being set. Inconclusive.");
        } else {
            out.add("  boot marker: NO REBOOT between the last two launches"
                    + " (same boot instant, uptime rose by "
                    + describe(elapsed - prevElapsed) + "). The SoC suspended and"
                    + " resumed, so BOOT_COMPLETED never fires and cannot help.");
        }
    }

    /**
     * Which third-party apps are sitting in the STOPPED state right now.
     *
     * <p>THE DIRECT TEST OF THE FORCE-STOP THEORY, and the reason this section
     * exists. The owner reports that after a sleep NOTHING third-party is left in
     * the app switcher — Carnyx, OsmAnd and Plexamp all gone. An ordinary
     * out-of-memory kill does not do that: it leaves the task in the switcher so
     * a tap can relaunch it. Wiping the task is what a force-stop does, and a
     * force-stopped package receives no broadcasts of any kind until a human
     * launches it by hand, which would explain — with one mechanism — why the
     * wake receiver, the sleep receiver and the runtime sleep watch have all
     * produced total silence rather than any of the failure lines they were
     * written to produce.
     *
     * <p>READ IT ON THE FIRST LAUNCH AFTER A WAKE, which is the only moment the
     * evidence exists. Carnyx cannot catch itself in the state — it is running,
     * so its own flag is necessarily clear — but the neighbours have not been
     * launched yet, and if OsmAnd or Plexamp come back STOPPED then the vendor
     * cleaner is the whole answer. Tap the row later in the drive, after the
     * driver has opened those apps, and the evidence is gone.
     *
     * <p>System packages are skipped. They are stopped and started for reasons of
     * their own and would bury the handful of lines that matter.
     */
    private static void stoppedApps(List<String> out) {
        try {
            PackageManager pm = ctx.getPackageManager();
            List<ApplicationInfo> all = pm.getInstalledApplications(0);
            int stopped = 0;
            int shown = 0;
            int third = 0;
            for (ApplicationInfo a : all) {
                if (a == null || a.packageName == null) {
                    continue;
                }
                boolean system = (a.flags & (ApplicationInfo.FLAG_SYSTEM
                        | ApplicationInfo.FLAG_UPDATED_SYSTEM_APP)) != 0;
                if (system) {
                    continue;
                }
                third++;
                if ((a.flags & FLAG_STOPPED) == 0) {
                    continue;
                }
                stopped++;
                if (shown < MAX_STOPPED) {
                    out.add("  STOPPED " + a.packageName);
                    shown++;
                }
            }
            // ZERO IS NOT A REFUTATION, and saying "nothing is force-stopped"
            // would read as one. A driver who taps this twenty minutes into a
            // drive has already opened the neighbours, which clears the flag on
            // every one of them, and the reading is then empty for a reason that
            // has nothing to do with the sleep.
            out.add("  stopped: " + stopped + " of " + third + " third-party packages"
                    + (stopped == 0
                        ? " — none right now, which only means something if this is"
                          + " the first launch after a wake and nothing else has been"
                          + " opened yet. Otherwise it is too late to tell."
                        : " — the vendor cleaner force-stops packages, which is why"
                          + " no broadcast of any kind reaches them"));
        } catch (Throwable t) {
            out.add("  stopped: unreadable (" + t.getClass().getSimpleName() + ")");
        }
    }

    /**
     * The two routes the SYSTEM keeps alive, rather than the app registering
     * something and hoping.
     *
     * <p>An accessibility service and a notification listener are both bound by
     * the platform from a list it holds in {@code Settings.Secure}, and the
     * platform re-binds them when they die. That is a categorically different
     * survival story from a broadcast receiver, and both carry an exemption from
     * the background-activity-start restriction that would otherwise refuse the
     * very launch this whole feature is for. #96 reached the same two by a
     * different road.
     *
     * <p>Read rather than assumed, and read EXPLICITLY rather than left to the
     * keyword sweep below, which would truncate the value at 120 characters and
     * could spend its whole budget before reaching them. Whether Carnyx appears in
     * either list is the line that matters: both need a one-time toggle by hand in
     * Settings, and an app update can drop the grant without saying so.
     */
    private static void survivable(List<String> out) {
        ContentResolver cr = ctx.getContentResolver();
        String me = ctx.getPackageName();
        read(out, cr, "accessibility_enabled", me);
        read(out, cr, "enabled_accessibility_services", me);
        read(out, cr, "enabled_notification_listeners", me);
    }

    /** One {@code Settings.Secure} row, saying whether we are in it. */
    private static void read(List<String> out, ContentResolver cr, String key, String me) {
        try {
            String v = Settings.Secure.getString(cr, key);
            if (v == null || v.isEmpty()) {
                out.add("  secure " + key + " = (empty)");
                return;
            }
            boolean mine = v.contains(me);
            out.add("  secure " + key + " = " + (v.length() > 160 ? v.substring(0, 160) + "…" : v)
                    + (mine ? "  <- CARNYX IS IN THIS LIST" : ""));
        } catch (Throwable t) {
            out.add("  secure " + key + ": unreadable (" + t.getClass().getSimpleName() + ")");
        }
    }

    /**
     * Who the platform believes can be STARTED by each of the broadcasts this app
     * has ever tried to wake on.
     *
     * <p>WHAT THIS SETTLES IS WHETHER THE ACTION STRINGS ARE REAL. Every wake
     * attempt so far has been built on {@code com.nwd.ACTION_OS_WAKE_UP}, a string
     * carried across from another project's notes about another unit's firmware.
     * If no component on this ROM — ours or the vendor's — resolves it, the string
     * is wrong and no amount of fixing the receiver will help. If vendor
     * components do resolve it, the action exists and the problem is delivery.
     *
     * <p>THIS IS A MANIFEST QUERY AND NOT A DELIVERY TEST. {@code
     * queryBroadcastReceivers} reports what packages DECLARE; it knows nothing
     * about the API-26 implicit-broadcast ban or the stopped state, both of which
     * bite later, at delivery. Seeing our own receiver listed here therefore
     * proves the manifest is right and proves nothing whatever about whether the
     * broadcast arrives — which is precisely the gap that made the last three
     * attempts look identical from the driver's seat.
     */
    private static void autoStart(List<String> out) {
        String[] actions = {
            Intent.ACTION_BOOT_COMPLETED,
            "com.nwd.ACTION_OS_WAKE_UP",
            "com.nwd.ACTION_ACCOFF_UPDATE",
            "com.nwd.action.ACTION_ACCOFF_UPDATE",
            "com.nwd.action.ACTION_KEY_VALUE",
        };
        String me = ctx.getPackageName();
        for (String action : actions) {
            try {
                PackageManager pm = ctx.getPackageManager();
                List<ResolveInfo> r = pm.queryBroadcastReceivers(new Intent(action), 0);
                if (r == null || r.isEmpty()) {
                    out.add("  action " + action + ": NOBODY declares a receiver");
                    continue;
                }
                StringBuilder b = new StringBuilder();
                b.append("  action ").append(action).append(": ").append(r.size());
                int shown = 0;
                for (ResolveInfo i : r) {
                    if (shown >= MAX_HANDLERS || i == null || i.activityInfo == null) {
                        continue;
                    }
                    String pkg = i.activityInfo.packageName;
                    b.append(shown == 0 ? " — " : ", ").append(pkg);
                    if (me.equals(pkg)) {
                        b.append("(us)");
                    }
                    shown++;
                }
                out.add(b.toString());
            } catch (Throwable t) {
                out.add("  action " + action + ": unreadable ("
                        + t.getClass().getSimpleName() + ")");
            }
        }
    }

    /** A duration a person can read, since every span here spans an ignition cycle. */
    private static String describe(long ms) {
        long s = Math.abs(ms) / 1000L;
        if (s < 90L) {
            return s + "s";
        }
        if (s < 5400L) {
            return (s / 60L) + "m";
        }
        if (s < 172800L) {
            return (s / 3600L) + "h";
        }
        return (s / 86400L) + "d";
    }

    /**
     * Doze's exemption, recorded so it can be ruled out.
     *
     * <p>It is NOT the thing being looked for: Doze is AOSP's idle policy and the
     * kill here happens on ACC-off, which is the vendor's. Worth a line because
     * "we already hold the exemption and are still killed" is the finding that
     * sends the search elsewhere.
     */
    private static void battery(List<String> out) {
        try {
            PowerManager pm = ctx.getSystemService(PowerManager.class);
            if (pm == null) {
                out.add("  battery: no PowerManager");
                return;
            }
            boolean exempt = pm.isIgnoringBatteryOptimizations(ctx.getPackageName());
            out.add("  battery: " + (exempt ? "exempt from Doze" : "NOT exempt from Doze"));
        } catch (Throwable t) {
            out.add("  battery: unreadable (" + t.getClass().getSimpleName() + ")");
        }
    }

    /**
     * The keyword sweep over the three settings tables.
     *
     * <p>A BUDGET PER TABLE, NOT ONE SHARED ACROSS ALL THREE. The budget used to
     * be spent in order — {@code MAX_SETTINGS - found} — so a table with many
     * matches consumed the whole allowance and the two after it reported nothing,
     * indistinguishable in the output from two tables that genuinely held nothing.
     * {@code global} is the broadest of the three and reliably matches on
     * {@code acc} and {@code background} alone, so the table most likely to hold a
     * vendor's private list was the one most likely to go unread.
     */
    private static void settings(List<String> out) {
        int found = 0;
        found += table(out, Settings.Global.CONTENT_URI, "global", MAX_SETTINGS);
        found += table(out, Settings.Secure.CONTENT_URI, "secure", MAX_SETTINGS);
        found += table(out, Settings.System.CONTENT_URI, "system", MAX_SETTINGS);
        if (found == 0) {
            out.add("  settings: nothing matched — which is not the same as no list");
        }
    }

    /**
     * One settings table, filtered by name.
     *
     * <p>Queried through the provider rather than by key, because the key is the
     * unknown: {@code Settings.Global.getString} answers only questions this code
     * already knows to ask, and the whole point is that the vendor's name for its
     * list is not known.
     */
    private static int table(List<String> out, Uri uri, String label, int budget) {
        if (budget <= 0) {
            return 0;
        }
        int hits = 0;
        try {
            ContentResolver cr = ctx.getContentResolver();
            try (Cursor c = cr.query(uri, new String[] { "name", "value" }, null, null, null)) {
                if (c == null) {
                    out.add("  " + label + ": not queryable");
                    return 0;
                }
                while (c.moveToNext() && hits < budget) {
                    String name = c.getString(0);
                    if (name == null || !matches(name)) {
                        continue;
                    }
                    String value = c.getString(1);
                    if (value != null && value.length() > 120) {
                        value = value.substring(0, 120) + "…(" + value.length() + ")";
                    }
                    out.add("  " + label + " " + name + " = " + value);
                    hits++;
                }
            }
        } catch (Throwable t) {
            out.add("  " + label + ": unreadable (" + t.getClass().getSimpleName() + ")");
        }
        return hits;
    }

    private static boolean matches(String name) {
        String n = name.toLowerCase(Locale.US);
        for (String k : KEYWORDS) {
            if (n.contains(k)) {
                return true;
            }
        }
        return false;
    }

    /**
     * Packages that plausibly own a keep-alive list, so the owner knows where to
     * look in the unit's own settings.
     *
     * <p>The vendor prefix first, because {@code com.nwd.*} is this ROM's own and
     * anything of theirs is a candidate; then anything whose name reads like a
     * power or launcher app, which is where such a list usually lives.
     */
    private static void packages(List<String> out) {
        try {
            PackageManager pm = ctx.getPackageManager();
            List<PackageInfo> all = pm.getInstalledPackages(0);
            int shown = 0;
            for (PackageInfo p : all) {
                if (p == null || p.packageName == null || shown >= MAX_PACKAGES) {
                    continue;
                }
                String n = p.packageName.toLowerCase(Locale.US);
                boolean interesting = n.startsWith("com.nwd.")
                        || n.contains("power") || n.contains("launcher")
                        || n.contains("settings") || n.contains("clean")
                        || n.contains("boot") || n.contains("whitelist");
                if (interesting) {
                    out.add("  pkg " + p.packageName);
                    shown++;
                }
            }
            out.add("  packages: " + all.size() + " visible"
                    + (Build.VERSION.SDK_INT >= 30 ? " (API 30+ filters this list)" : ""));
        } catch (Throwable t) {
            out.add("  packages: unreadable (" + t.getClass().getSimpleName() + ")");
        }
    }

    private static String join(List<String> lines) {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < lines.size(); i++) {
            if (i > 0) {
                b.append('\n');
            }
            b.append(lines.get(i));
        }
        Log.i(TAG, "report: " + lines.size() + " lines");
        return b.toString();
    }
}
