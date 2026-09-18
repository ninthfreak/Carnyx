package com.ninthfreak.carnyx;

import android.content.Context;
import android.content.Intent;
import android.util.Log;

import org.xmlpull.v1.XmlPullParser;
import org.xmlpull.v1.XmlPullParserFactory;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.StringReader;
import java.util.ArrayList;
import java.util.List;

/**
 * Install the kernel source remap by asking the vendor's own file copier to
 * write it, from an unprivileged app.
 *
 * <h2>WHAT THIS IS FOR</h2>
 *
 * <p>The head unit's kernel ({@code com.nwd.kernel}) launches the stock radio
 * app on wake by restoring the last audio source. Before it does, it consults
 * {@code /config/app/replace_source_list.xml} and, for any {@code appid} listed
 * there, launches THAT package instead. Radio is appid 8. So dropping a file
 * that maps appid 8 to Carnyx makes the kernel launch Carnyx in the stock app's
 * place — #133 outcome A. See {@code docs/vendor/README.md}.
 *
 * <p>{@code /config} is a system partition an installed app cannot write. But
 * {@code com.nwd.factory.setting} exports {@code CopyFileService} — EXPORTED, NO
 * PERMISSION, NO CALLER CHECK — a config-driven copier whose config names the
 * destination. This class stages a payload and asks that service to copy it into
 * {@code /config/app}.
 *
 * <h2>THE PAYLOAD SHAPE IS NOT ARBITRARY, IT IS READ OFF THE COPIER</h2>
 *
 * <p>{@code CopyFileThread} routes a config with ONE {@code PathItem} through
 * {@code ParserXMLFile}, which copies the CONTENTS OF A SOURCE DIRECTORY into
 * the destination directory. A lone FILE source that is not {@code update.zip}
 * is logged and skipped; the {@code update.zip} branch fires a
 * {@code MASTER_CLEAR} factory reset. So the payload is a DIRECTORY copy, which
 * reaches neither of those:
 *
 * <pre>
 *   &lt;COPY_PATH&gt;/                     the staging root, passed as an extra
 *     CopyFileConfig.xml             &lt;PathItem PathSrc="/payload" PathDes="/config/app"/&gt;
 *     payload/
 *       replace_source_list.xml      the remap, copied to /config/app/ by name
 * </pre>
 *
 * <p>NO {@code autocopy} MARKER, DELIBERATELY. Its presence would skip the
 * copier's confirmation dialog — and in the file/{@code update.zip} branch it is
 * the trigger for the {@code MASTER_CLEAR}. Leaving it out keeps this a country
 * mile from that path AND means the copy only proceeds when the driver taps OK
 * on the vendor's own dialog. The install button is one tap; the dialog is the
 * second, and the second is the vendor asking "really write the system
 * partition?", which is a question worth leaving in.
 *
 * <h2>THE ONE THING THIS CANNOT GUARANTEE</h2>
 *
 * <p>Whether the factory process can actually write {@code /config} — it is not
 * {@code android.uid.system}, so success rests on that partition's permissions
 * on the running unit, which no decompile shows. {@link #verify} reads the file
 * back and reports the truth either way; if it never appears, the route was not
 * available and the USB/root fallback in {@code docs/vendor/} remains.
 */
final class CarnyxRemap {

    private static final String TAG = "CarnyxRemap";

    /** The unguarded factory copier. */
    private static final String FACTORY_PKG = "com.nwd.factory.setting";
    private static final String COPY_SERVICE = "com.nwd.factory.copy.CopyFileService";

    /** Where the kernel reads the remap. {@code /config} is the default
     *  {@code ro.nwd.config.path}; the copier makes the dir if absent. */
    private static final String CONFIG_APP_DIR = "/config/app";
    private static final String REMAP_NAME = "replace_source_list.xml";
    private static final String DEST = CONFIG_APP_DIR + "/" + REMAP_NAME;

    /**
     * Where the ORIGINAL goes before it is replaced.
     *
     * <p>ALONGSIDE THE FILE IT BACKS UP, in {@code /config/app}, and not in this
     * app's own storage: a reinstall wipes app storage and the one copy of a
     * vendor-configured remap list would go with it. Written once — see the
     * guard in {@link #install} — so a second install cannot back up its own
     * output over the true original.
     */
    private static final String BACKUP_NAME = "replace_source_list.xml.carnyx-backup";

    /** {@code SourceConstant.APPID_RADIO}. NOT the MCU's source id 4. */
    private static final int RADIO_APPID = 8;

    private static final String SELF_PKG = "com.ninthfreak.carnyx";
    private static final String SELF_CLASS = "android.app.NativeActivity";

    /** The staging layout under the app's external files dir. */
    private static final String STAGE_DIR = "carnyx-remap";
    private static final String PAYLOAD_SUBDIR = "payload";
    private static final String CONFIG_NAME = "CopyFileConfig.xml";

    /** How long to watch {@link #DEST} for the copier's write, in milliseconds.
     *  Generous because the copy waits on a human tapping the vendor's
     *  confirmation dialog, and a driver may be looking at the radio first. */
    private static final long WATCH_MS = 120_000L;

    /** How often to look. Cheap — one {@code stat} and a small read. */
    private static final long POLL_MS = 500L;

    private static Context ctx;

    /**
     * What {@link #DEST} held before this session's install, and whether it was
     * there at all.
     *
     * <p>THE "BEFORE" HALF OF THE ANSWER. Reading the destination to merge into
     * it already tells us what was there; keeping it is what lets the watcher and
     * {@link #verify} say whether anything actually CHANGED, rather than only
     * what the file says now. "It reads as installed" and "we installed it" are
     * different claims, and only the second one answers whether the copier could
     * write {@code /config}.
     *
     * <p>IN-SESSION, NOT DURABLE. The verdict is pushed into the diagnostics log
     * as soon as it is known, so the driver does not have to be holding this
     * state to see it. Persisting it would mean a fourth durable note ring
     * across the class-loader divide for a case — process death between the two
     * taps — that the log line already covers.
     */
    private static String beforeBody;

    private static boolean beforeExisted;

    /** One watcher at a time. See {@link #startWatcher}. */
    private static volatile boolean watching;

    private CarnyxRemap() {}

    /**
     * Java → Rust: put a line in the diagnostics log from the watcher thread.
     *
     * <p>Registered by hand in `src/android/remap.rs`, as `CarnyxLocation`'s
     * `nativeNote` is. The watcher finishes long after the tap that started it
     * returned, so it has no return value to travel back on — this is the
     * channel, and on this unit the diagnostics panel is the only one a driver
     * can read.
     */
    private static native void nativeRemapNote(String line);

    /**
     * One {@code ReplaceSourceItem}, holding exactly what the kernel reads.
     *
     * <p>{@code ReplaceSourceList.loadConfig} takes three attributes per item and
     * nothing else — {@code appid}, {@code pkgName}, {@code className} — so these
     * three are the whole of what a merge has to carry across.
     */
    private static final class Item {
        final int appid;
        final String pkg;
        final String cls;

        Item(int appid, String pkg, String cls) {
            this.appid = appid;
            this.pkg = pkg;
            this.cls = cls;
        }
    }

    /**
     * Take the context. Safe to call more than once.
     *
     * @return a line for the diagnostics log, never null.
     */
    static synchronized String attach(Context context) {
        if (context == null) {
            return "remap: no context";
        }
        if (ctx == null) {
            ctx = context.getApplicationContext();
        }
        return "remap: ready";
    }

    /**
     * Merge our entry into whatever is already there and ask the factory copier
     * to write it to {@code /config/app}.
     *
     * <h2>IT READS THE DESTINATION FIRST, AND THE FIRST CUT DID NOT</h2>
     *
     * <p>{@code ReplaceSourceList} is a LIST — the kernel iterates every
     * {@code ReplaceSourceItem} in the file and remaps each one — and the vendor
     * copier is destructive: {@code CopyFile} runs
     * {@code if (dst.exists()) dst.delete()} before it writes. So staging a
     * single-entry file and firing the copier blind would DELETE any other
     * source remap the unit already had, silently, with nothing to restore from.
     * The file exists precisely so an integrator can repoint sources at their own
     * apps, so assuming it is ours to replace is the wrong shape of edit.
     *
     * <p>WHAT IT DOES INSTEAD. Read {@link #DEST}; then
     *
     * <ul>
     *   <li>absent — write ours alone;
     *   <li>already maps appid 8 to Carnyx — do nothing at all, and say so;
     *   <li>present with other entries — keep every one of them, replace only the
     *       appid 8 entry, and stage a byte-exact backup of the original beside
     *       the merged file;
     *   <li>present but unreadable, or malformed — STOP. Overwriting a file this
     *       cannot see is the one move to refuse.
     * </ul>
     *
     * <p>THE REBUILD IS LOSSLESS FOR EVERYTHING THE KERNEL READS.
     * {@code loadConfig} takes exactly three attributes per item — {@code appid},
     * {@code pkgName}, {@code className} — so a file rebuilt from those is
     * functionally identical. Comments and formatting are not preserved, which is
     * why the original goes to {@link #BACKUP_NAME} rather than being trusted to
     * this method's own output.
     *
     * @return one line for the diagnostics log, never null.
     */
    static synchronized String install() {
        if (ctx == null) {
            return "remap install: no context";
        }

        // ── READ WHAT IS THERE BEFORE DECIDING TO WRITE ──────────────────────
        File dest = new File(DEST);
        String existing = null;
        if (dest.exists()) {
            if (!dest.canRead()) {
                return "remap install: REFUSED — " + DEST
                        + " exists but cannot be read here, and overwriting a file"
                        + " this cannot see would discard whatever is in it";
            }
            try {
                existing = readFile(dest);
            } catch (Throwable t) {
                return "remap install: REFUSED — could not read the existing " + DEST + " — " + t;
            }
        }

        List<Item> keep = new ArrayList<Item>();
        if (existing != null) {
            List<Item> found;
            try {
                found = parseItems(existing);
            } catch (Throwable t) {
                return "remap install: REFUSED — " + DEST
                        + " is there but could not be parsed, so its entries cannot be"
                        + " preserved (" + t + ")";
            }
            for (int i = 0; i < found.size(); i++) {
                Item it = found.get(i);
                if (it.appid == RADIO_APPID) {
                    // OURS ALREADY? Then there is nothing to write and no reason
                    // to touch a system partition.
                    if (SELF_PKG.equals(it.pkg)) {
                        return "remap install: already installed — " + DEST
                                + " maps the radio source to Carnyx, left alone";
                    }
                    // Somebody else's radio remap. Replaced, which IS the errand,
                    // and named so the swap is on the record.
                    Log.i(TAG, "replacing radio remap " + it.pkg);
                    continue;
                }
                keep.add(it);
            }
        }
        keep.add(new Item(RADIO_APPID, SELF_PKG, SELF_CLASS));

        File root;
        int kept = keep.size() - 1;
        boolean backing = false;
        try {
            // ── STAGED IN DOWNLOADS, NOT IN THIS APP'S OWN DIRECTORY ─────────
            //
            // MEASURED 2026-09-18. The first cut staged in
            // `getExternalFilesDir(null)`, which lives under
            // `/storage/emulated/0/Android/data/<pkg>/`, and the copier reported
            // a missing path and wrote nothing: Android 10 walls that subtree off
            // from OTHER apps, and the factory app — a different uid — could not
            // see the payload it was pointed at. The staging directory has to be
            // somewhere a foreign process can read.
            //
            // DOWNLOADS IS PROVEN ON THIS UNIT. `NwdBridge.writeLog` puts the
            // diagnostics log there through MediaStore and the owner takes those
            // files off the device, so it is writable by us and readable as an
            // ordinary path — which is exactly what the factory app, legacy
            // storage on targetSdk 19, needs.
            File downloads = android.os.Environment.getExternalStoragePublicDirectory(
                    android.os.Environment.DIRECTORY_DOWNLOADS);
            if (downloads == null) {
                return "remap install: no Downloads directory to stage the payload";
            }
            root = new File(downloads, STAGE_DIR);
            String rel = android.os.Environment.DIRECTORY_DOWNLOADS + "/" + STAGE_DIR;
            String relPayload = rel + "/" + PAYLOAD_SUBDIR;

            writeShared(relPayload, REMAP_NAME, mergedXml(keep));
            // THE BACKUP GOES TO /config/app TOO, not just to Downloads, so it
            // outlives a reinstall — and it is written ONLY when there is no
            // backup there already, because a second install would otherwise
            // back up its own merged output over the true original.
            if (existing != null && !new File(CONFIG_APP_DIR, BACKUP_NAME).exists()) {
                writeShared(relPayload, BACKUP_NAME, existing);
                backing = true;
            } else {
                // A backup staged by a PREVIOUS attempt would otherwise be copied
                // again by this one. The copier takes everything in the payload
                // directory, so what is not wanted has to be removed.
                deleteShared(relPayload, BACKUP_NAME);
            }
            writeShared(rel, CONFIG_NAME, copyConfigXml());
            // WORLD-READABLE, best effort. Files written through MediaStore are
            // owned by the media provider and these calls may do nothing; the
            // watcher is what decides whether it actually worked.
            makeReadable(root);
            makeReadable(new File(root, PAYLOAD_SUBDIR));
            makeReadable(new File(new File(root, PAYLOAD_SUBDIR), REMAP_NAME));
            makeReadable(new File(root, CONFIG_NAME));
        } catch (Throwable t) {
            return "remap install: staging failed — " + t;
        }
        try {
            Intent i = new Intent();
            i.setClassName(FACTORY_PKG, COPY_SERVICE);
            i.putExtra("COPY_PATH", root.getAbsolutePath());
            ctx.startService(i);
        } catch (Throwable t) {
            // The likely ones: the package is not visible (needs the <queries>
            // entry), or the service is not present on this unit.
            return "remap install: could not reach " + FACTORY_PKG + " — " + t;
        }
        // ── THE "BEFORE" IS KEPT, AND THEN WATCHED FOR ───────────────────────
        //
        // Asking the copier is not the same as the copier succeeding, and the
        // first cut of this returned "asked…" and left the driver to tap Check
        // at a moment of their own choosing. The copy lands whenever the vendor
        // dialog is confirmed, so there is no moment this method could
        // sensibly check for itself — a watcher can, and it reports the verdict
        // into the log the instant it knows.
        beforeBody = existing;
        beforeExisted = existing != null;
        startWatcher();
        // THE STAGING PATH IS IN THE LINE, because the 2026-09-18 failure was
        // about that path and the log could not show which one had been handed
        // over. If the copier reports a missing path again, this says what it was
        // given.
        return "remap install: staged in " + root.getAbsolutePath() + ", asked "
                + FACTORY_PKG + " to write " + REMAP_NAME + " into " + CONFIG_APP_DIR
                + (existing == null ? " (no file there before)"
                        : " (was " + kept + " other entr" + (kept == 1 ? "y" : "ies")
                                + (backing ? ", backed up" : "") + ")")
                + " — confirm the copy dialog if it appears; the result posts here itself";
    }

    /**
     * Watch {@link #DEST} until it changes, or until {@link #WATCH_MS} is up.
     *
     * <p>ON ITS OWN THREAD, because the thing it waits for is a human tapping a
     * dialog in another app. A daemon thread so it can never hold the process
     * open, and one at a time so repeated taps do not stack watchers.
     *
     * <p>WHAT IT CAN SAY THAT A READ CANNOT. "The file names Carnyx" is
     * ambiguous — it could have said that before this app ever ran. Comparing
     * against the recorded before-state separates the three answers that matter:
     * the copier wrote what we asked, the copier wrote something else, or
     * nothing changed at all — which is the signature of {@code /config} not
     * being writable by the factory process, the one thing the firmware could
     * not tell us.
     */
    private static void startWatcher() {
        if (watching) {
            return;
        }
        watching = true;
        Thread t = new Thread(new Runnable() {
            @Override public void run() {
                try {
                    long deadline = System.currentTimeMillis() + WATCH_MS;
                    while (System.currentTimeMillis() < deadline) {
                        try {
                            Thread.sleep(POLL_MS);
                        } catch (InterruptedException e) {
                            return;
                        }
                        String now = currentBody();
                        boolean exists = new File(DEST).exists();
                        if (exists != beforeExisted
                                || (now != null && !now.equals(beforeBody))) {
                            post("radio takeover: " + describe(now, exists));
                            return;
                        }
                    }
                    post("radio takeover: nothing changed at " + DEST + " in "
                            + (WATCH_MS / 1000) + "s — "
                            + (beforeExisted ? "it still reads as it did before the tap"
                                    : "the file was never created")
                            + ". Either the copy dialog was not confirmed, or the factory"
                            + " app cannot write /config on this unit");
                } finally {
                    watching = false;
                }
            }
        }, "carnyx-remap-watch");
        t.setDaemon(true);
        t.start();
    }

    /** {@link #DEST}'s contents, or null when absent or unreadable. */
    private static String currentBody() {
        File f = new File(DEST);
        if (!f.exists() || !f.canRead()) {
            return null;
        }
        try {
            return readFile(f);
        } catch (Throwable t) {
            return null;
        }
    }

    /** What a body reads as: installed, something else, or gone. */
    private static String describe(String body, boolean exists) {
        if (!exists) {
            return DEST + " was deleted and not replaced";
        }
        if (body == null) {
            return DEST + " changed but cannot be read here";
        }
        List<Item> items;
        try {
            items = parseItems(body);
        } catch (Throwable t) {
            return DEST + " changed but will not parse — the kernel loads no remaps"
                    + " in this state (" + t + ")";
        }
        int others = 0;
        Item radio = null;
        for (int i = 0; i < items.size(); i++) {
            if (items.get(i).appid == RADIO_APPID) {
                radio = items.get(i);
            } else {
                others++;
            }
        }
        String also = others == 0 ? "" : ", " + others + " other entr"
                + (others == 1 ? "y" : "ies") + " kept";
        if (radio == null) {
            return DEST + " changed but has no appid " + RADIO_APPID + " entry" + also;
        }
        if (SELF_PKG.equals(radio.pkg)) {
            return "INSTALLED — the copier wrote it; the radio source now launches Carnyx"
                    + also;
        }
        return DEST + " changed but the radio source points at " + radio.pkg + also;
    }

    /** One line to the diagnostics log, or to logcat if the native is not bound. */
    private static void post(String line) {
        try {
            nativeRemapNote(line);
        } catch (Throwable t) {
            // A host build or a dex whose natives were never registered. The
            // line is still worth having somewhere.
            Log.i(TAG, line);
        }
    }

    /**
     * Read {@code /config/app/replace_source_list.xml} back and say what is there.
     *
     * @return one line for the diagnostics log, never null.
     */
    static synchronized String verify() {
        File f = new File(DEST);
        try {
            if (!f.exists()) {
                return "remap check: NOT installed — " + DEST + " is not present";
            }
            if (!f.canRead()) {
                return "remap check: " + DEST + " exists but is not readable here";
            }
            String body = readFile(f);
            // PARSED, NOT PATTERN-MATCHED. A substring search answers "does this
            // file mention Carnyx", which is not the question — the question is
            // whether the RADIO entry points at us, and how many other entries
            // came through the merge intact.
            String tail = new File(CONFIG_APP_DIR, BACKUP_NAME).exists()
                    ? "; the original is saved as " + BACKUP_NAME
                    : "";
            List<Item> items;
            try {
                items = parseItems(body);
            } catch (Throwable t) {
                return "remap check: " + DEST + " is present but will not parse — the kernel"
                        + " loads no remaps at all in this state (" + t + ")" + tail;
            }
            int others = 0;
            Item radio = null;
            for (int i = 0; i < items.size(); i++) {
                if (items.get(i).appid == RADIO_APPID) {
                    radio = items.get(i);
                } else {
                    others++;
                }
            }
            String also = others == 0 ? "" : ", " + others + " other entr"
                    + (others == 1 ? "y" : "ies") + " kept";
            if (radio == null) {
                return "remap check: NOT installed — " + DEST + " has no appid "
                        + RADIO_APPID + " entry" + also + tail;
            }
            // ── AND WHETHER THIS SESSION'S INSTALL IS WHAT PUT IT THERE ──────
            //
            // Only sayable when an install ran in this process. Without it the
            // row reports the state and not the CAUSE, and "it was already like
            // this" reads identically to "we just did it" — which is the
            // difference between knowing /config is writable and assuming it.
            String since = "";
            if (beforeExisted || beforeBody != null) {
                since = body.equals(beforeBody)
                        ? "; unchanged since this session's install — the copier did not write"
                        : "; changed since this session's install";
            } else if (watching) {
                since = "; still watching for the copier's write";
            }
            if (SELF_PKG.equals(radio.pkg)) {
                return "remap check: INSTALLED — " + DEST + " maps the radio source to Carnyx"
                        + also + since + tail;
            }
            return "remap check: NOT installed — the radio source is mapped to "
                    + radio.pkg + also + since + tail;
        } catch (Throwable t) {
            return "remap check: could not read " + DEST + " — " + t;
        }
    }

    /**
     * The copier config. ONE {@code PathItem}, DIRECTORY to DIRECTORY.
     *
     * <p>{@code PathSrc} is relative to {@code COPY_PATH} and names the payload
     * subdir the copier lists; {@code PathDes} is the absolute destination
     * directory. See the class doc for why a directory copy and not a file one.
     */
    private static String copyConfigXml() {
        return "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n"
                + "<CopyConfig>\n"
                + "    <PathItem PathSrc=\"/" + PAYLOAD_SUBDIR + "\" PathDes=\"" + CONFIG_APP_DIR + "\" />\n"
                + "</CopyConfig>\n";
    }

    /**
     * Every {@code ReplaceSourceItem} the kernel would read out of {@code xml}.
     *
     * <p>STRICT ON PURPOSE. An entry with a missing attribute or a non-numeric
     * {@code appid} throws rather than being skipped, and {@link #install} turns
     * that into a refusal. The kernel's own {@code loadConfig} wraps the whole
     * parse in one try, so a malformed entry there means NO remap loads at all —
     * a file in that state is already broken, and quietly rebuilding it into
     * something different would be this method inventing the driver's
     * configuration.
     */
    private static List<Item> parseItems(String xml) throws Exception {
        List<Item> out = new ArrayList<Item>();
        XmlPullParser p = XmlPullParserFactory.newInstance().newPullParser();
        p.setInput(new StringReader(xml));
        int ev = p.getEventType();
        while (ev != XmlPullParser.END_DOCUMENT) {
            if (ev == XmlPullParser.START_TAG && "ReplaceSourceItem".equals(p.getName())) {
                // NULL NAMESPACE, which is what the kernel passes too — its
                // decompile reads `getAttributeValue(0, "appid")`.
                String appid = p.getAttributeValue(null, "appid");
                String pkg = p.getAttributeValue(null, "pkgName");
                String cls = p.getAttributeValue(null, "className");
                if (appid == null || pkg == null || cls == null) {
                    throw new IOException("a ReplaceSourceItem is missing appid, pkgName or className");
                }
                out.add(new Item(Integer.parseInt(appid.trim()), pkg, cls));
            }
            ev = p.next();
        }
        return out;
    }

    /** The file to write: every kept entry, ours among them. */
    private static String mergedXml(List<Item> items) {
        StringBuilder b = new StringBuilder();
        b.append("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        b.append("<!-- Written by Carnyx (").append(SELF_PKG).append("): map radio source appid ")
                .append(RADIO_APPID).append(" to Carnyx.\n");
        b.append("     Entries for other sources are carried over; the file this replaced,\n");
        b.append("     if there was one, is beside this as ").append(BACKUP_NAME).append(". -->\n");
        b.append("<ReplaceSourceList>\n");
        for (int i = 0; i < items.size(); i++) {
            Item it = items.get(i);
            b.append("    <ReplaceSourceItem appid=\"").append(it.appid)
                    .append("\" pkgName=\"").append(esc(it.pkg))
                    .append("\" className=\"").append(esc(it.cls))
                    .append("\" />\n");
        }
        b.append("</ReplaceSourceList>\n");
        return b.toString();
    }

    /** Attribute-safe. Package and class names should never need it; correctness
     *  here is cheaper than trusting that about somebody else's file. */
    private static String esc(String s) {
        return s.replace("&", "&amp;")
                .replace("<", "&lt;")
                .replace(">", "&gt;")
                .replace("\"", "&quot;");
    }

    /**
     * Write one file into shared storage, where another app can read it.
     *
     * <p>THROUGH MediaStore ON API 29+, because a targetSdk 34 app cannot open a
     * {@code FileOutputStream} anywhere in shared storage — and the app-specific
     * directory it CAN write is the one the factory app cannot read. MediaStore
     * creates the directories named by {@code rel} on the way.
     *
     * <p>DELETE FIRST, ALWAYS. A second insert with the same display name does
     * not replace the first, it creates {@code name (1).xml} beside it — and the
     * copier copies every file in the payload directory, so the stale one would
     * travel too.
     *
     * @param rel the relative directory, e.g. {@code Download/carnyx-remap/payload}
     */
    private static void writeShared(String rel, String name, String body) throws IOException {
        byte[] bytes = body.getBytes("utf-8");
        if (android.os.Build.VERSION.SDK_INT >= 29) {
            deleteShared(rel, name);
            android.content.ContentResolver resolver = ctx.getContentResolver();
            android.content.ContentValues values = new android.content.ContentValues();
            values.put(android.provider.MediaStore.Downloads.DISPLAY_NAME, name);
            values.put(android.provider.MediaStore.Downloads.MIME_TYPE, "text/xml");
            values.put(android.provider.MediaStore.Downloads.RELATIVE_PATH, rel);
            android.net.Uri uri = resolver.insert(
                    android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
            if (uri == null) {
                throw new IOException("MediaStore insert returned null for " + rel + "/" + name);
            }
            java.io.OutputStream out = resolver.openOutputStream(uri);
            if (out == null) {
                throw new IOException("openOutputStream returned null for " + rel + "/" + name);
            }
            try {
                out.write(bytes);
            } finally {
                out.close();
            }
            return;
        }
        // Below 29 the public directory is writable directly. Unused on this
        // unit, kept so the class is not silently API-29-only.
        File dir = new File(android.os.Environment.getExternalStorageDirectory(), rel);
        if (!dir.exists() && !dir.mkdirs()) {
            throw new IOException("could not create " + dir);
        }
        File f = new File(dir, name);
        writeFile(f, body);
        makeReadable(f);
    }

    /** Remove a staged file, so a previous run's leftovers are not copied. */
    private static void deleteShared(String rel, String name) {
        try {
            if (android.os.Build.VERSION.SDK_INT >= 29) {
                // RELATIVE_PATH is stored with a trailing separator.
                ctx.getContentResolver().delete(
                        android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI,
                        android.provider.MediaStore.Downloads.RELATIVE_PATH + "=? AND "
                                + android.provider.MediaStore.Downloads.DISPLAY_NAME + "=?",
                        new String[] {rel + "/", name});
                return;
            }
            File f = new File(new File(android.os.Environment.getExternalStorageDirectory(), rel), name);
            if (f.exists() && !f.delete()) {
                Log.w(TAG, "could not delete stale " + f);
            }
        } catch (Throwable t) {
            Log.w(TAG, "could not clear stale " + rel + "/" + name + ": " + t);
        }
    }

    private static void writeFile(File f, String body) throws IOException {
        FileOutputStream os = new FileOutputStream(f);
        try {
            os.write(body.getBytes("utf-8"));
            os.flush();
        } finally {
            os.close();
        }
    }

    private static String readFile(File f) throws IOException {
        FileInputStream is = new FileInputStream(f);
        try {
            ByteArrayOutputStream bos = new ByteArrayOutputStream();
            byte[] buf = new byte[4096];
            int n;
            while ((n = is.read(buf)) != -1) {
                bos.write(buf, 0, n);
            }
            return new String(bos.toByteArray(), "utf-8");
        } finally {
            is.close();
        }
    }

    private static void makeReadable(File f) {
        try {
            f.setReadable(true, false);
            f.setExecutable(true, false);
        } catch (Throwable t) {
            Log.w(TAG, "could not widen permissions on " + f + ": " + t);
        }
    }
}
