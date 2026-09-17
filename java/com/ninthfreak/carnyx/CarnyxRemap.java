package com.ninthfreak.carnyx;

import android.content.Context;
import android.content.Intent;
import android.util.Log;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;

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

    /** The staging layout under the app's external files dir. */
    private static final String STAGE_DIR = "carnyx-remap";
    private static final String PAYLOAD_SUBDIR = "payload";
    private static final String CONFIG_NAME = "CopyFileConfig.xml";

    /** The remap itself. Kept in sync with {@code docs/vendor/replace_source_list.xml}. */
    private static final String REMAP_XML =
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n"
            + "<!-- Written by Carnyx (com.ninthfreak.carnyx): map radio source appid 8 to Carnyx. -->\n"
            + "<ReplaceSourceList>\n"
            + "    <ReplaceSourceItem appid=\"8\" pkgName=\"com.ninthfreak.carnyx\""
            + " className=\"android.app.NativeActivity\" />\n"
            + "</ReplaceSourceList>\n";

    private static Context ctx;

    private CarnyxRemap() {}

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
     * Stage the payload and ask the factory copier to write it to {@code /config/app}.
     *
     * @return one line for the diagnostics log, never null.
     */
    static synchronized String install() {
        if (ctx == null) {
            return "remap install: no context";
        }
        File root;
        try {
            File ext = ctx.getExternalFilesDir(null);
            if (ext == null) {
                return "remap install: no external storage to stage the payload";
            }
            root = new File(ext, STAGE_DIR);
            File payload = new File(root, PAYLOAD_SUBDIR);
            if (!payload.exists() && !payload.mkdirs()) {
                return "remap install: could not create " + payload;
            }
            writeFile(new File(payload, REMAP_NAME), REMAP_XML);
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
        return "remap install: asked " + FACTORY_PKG + " to copy " + REMAP_NAME
                + " into " + CONFIG_APP_DIR + " — confirm the copy dialog if it appears, then Check";
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
            boolean ours = body.contains("com.ninthfreak.carnyx");
            boolean appid8 = body.contains("appid=\"8\"") || body.contains("appid='8'");
            if (ours && appid8) {
                return "remap check: INSTALLED — " + DEST + " maps the radio source to Carnyx";
            }
            if (ours) {
                return "remap check: " + DEST + " names Carnyx but not appid 8 — check the file";
            }
            return "remap check: a " + REMAP_NAME + " exists but does not name Carnyx";
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
