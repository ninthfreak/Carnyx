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

    private static Context ctx;

    private CarnyxRemap() {}

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
            File ext = ctx.getExternalFilesDir(null);
            if (ext == null) {
                return "remap install: no external storage to stage the payload";
            }
            root = new File(ext, STAGE_DIR);
            File payload = new File(root, PAYLOAD_SUBDIR);
            // CLEARED EACH RUN. The copier copies whatever is in this directory,
            // so a file staged by a previous attempt would be copied again —
            // including a backup this run decided not to make.
            clearDir(payload);
            if (!payload.exists() && !payload.mkdirs()) {
                return "remap install: could not create " + payload;
            }
            writeFile(new File(payload, REMAP_NAME), mergedXml(keep));
            // THE BACKUP GOES TO /config/app TOO, not just to this app's storage,
            // so it outlives a reinstall — and it is written ONLY when there is
            // no backup there already, because a second install would otherwise
            // back up its own merged output over the true original.
            if (existing != null && !new File(CONFIG_APP_DIR, BACKUP_NAME).exists()) {
                writeFile(new File(payload, BACKUP_NAME), existing);
                makeReadable(new File(payload, BACKUP_NAME));
                backing = true;
            }
            writeFile(new File(root, CONFIG_NAME), copyConfigXml());
            // WORLD-READABLE, because the factory app is a different uid and has
            // to traverse and read these. Best effort — the verify step is what
            // actually decides whether it worked.
            makeReadable(root);
            makeReadable(payload);
            makeReadable(new File(payload, REMAP_NAME));
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
        return "remap install: asked " + FACTORY_PKG + " to write " + REMAP_NAME
                + " into " + CONFIG_APP_DIR
                + (existing == null ? " (new file)" : " (kept " + kept + " existing entr"
                        + (kept == 1 ? "y" : "ies") + (backing ? ", backed up" : "") + ")")
                + " — confirm the copy dialog if it appears, then Check";
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
            if (SELF_PKG.equals(radio.pkg)) {
                return "remap check: INSTALLED — " + DEST + " maps the radio source to Carnyx"
                        + also + tail;
            }
            return "remap check: NOT installed — the radio source is mapped to "
                    + radio.pkg + also + tail;
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

    /** Empty the staging directory, so only this run's files are copied. */
    private static void clearDir(File dir) {
        File[] kids = dir.listFiles();
        if (kids == null) {
            return;
        }
        for (int i = 0; i < kids.length; i++) {
            if (!kids[i].delete()) {
                Log.w(TAG, "could not clear stale payload file " + kids[i]);
            }
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
