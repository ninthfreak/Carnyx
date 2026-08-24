package com.ninthfreak.carnyx;

import android.content.ContentResolver;
import android.content.Context;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.PowerManager;
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
        "keepalive", "keep_alive", "freeze", "hibernat", "standby", "doze",
        "background", "bg_", "powersave", "power_save", "clean", "killer",
        "acc", "sleep", "wake", "boot",
    };

    /** The ring holds 200 lines and this is one of several writers. */
    private static final int MAX_SETTINGS = 40;
    private static final int MAX_PACKAGES = 25;

    private static Context ctx;

    private CarnyxKeepAlive() {
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
            return "keep-alive probe: no context";
        }
        out.add("keep-alive probe: android " + Build.VERSION.SDK_INT
                + ", " + Build.MANUFACTURER + " " + Build.MODEL);
        battery(out);
        settings(out);
        packages(out);
        return join(out);
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

    private static void settings(List<String> out) {
        int found = 0;
        found += table(out, Settings.Global.CONTENT_URI, "global", MAX_SETTINGS - found);
        found += table(out, Settings.Secure.CONTENT_URI, "secure", MAX_SETTINGS - found);
        found += table(out, Settings.System.CONTENT_URI, "system", MAX_SETTINGS - found);
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
