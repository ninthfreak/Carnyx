package com.ninthfreak.carnyx;

import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.ServiceConnection;
import android.os.IBinder;
import android.os.Parcel;
import android.util.Log;

/**
 * A blocking path to the MCU, for handing the FM source back before the unit
 * sleeps.
 *
 * <h2>WHY THIS EXISTS: THE BROADCAST CANNOT WIN THE RACE</h2>
 *
 * <p>{@code NwdBridge.releaseSource} ends in {@code ctx.sendBroadcast} of
 * {@code ACTION_REQUEST_CHANGE_SOURCE}. That broadcast has NO receiver in the
 * radio app or the radio service — it is consumed by a THIRD process,
 * {@code com.nwd.kernel}. So the release has to be queued by ActivityManager,
 * dispatched, and delivered across a process boundary.
 *
 * <p>Measured on the unit: that works when the driver closes Carnyx by hand and
 * does NOT work at ACC-off. The system stays awake for the first and suspends
 * before delivery on the second. Same code, opposite outcome.
 *
 * <p>This class is the other way in. It binds that third process directly and
 * hands it the MCU frame, and the whole chain is synchronous:
 *
 * <pre>
 *     IKernelFeature.request(byte[])        transaction 1, no FLAG_ONEWAY
 *       -&gt; ProtocalUtil.writeDataToMCU
 *         -&gt; ICommunicator.writeData       the UART
 * </pre>
 *
 * <p>No handler, no queue, no worker thread anywhere in it. The bytes are on the
 * serial port before {@code request} returns. See {@code docs/vendor/README.md}
 * for the decompile this is read from, and {@code docs/TASKS.md} #133.
 *
 * <h2>RAW {@code transact} RATHER THAN A GENERATED STUB</h2>
 *
 * <p>One transaction, one argument. Adding {@code IKernelFeature.aidl} to the
 * build would generate a proxy for six methods to use one of them, and would put
 * another vendor interface description in this tree. The descriptor string and
 * the transaction number below ARE the interface as far as this app is
 * concerned, and they sit next to the evidence that established them.
 *
 * <h2>BOUND EARLY, HELD OPEN</h2>
 *
 * <p>{@code bindService} is asynchronous — the connection callback arrives on the
 * main looper — and the moment this class exists to serve is the one where there
 * is no time left to start binding. So the bind happens at attach and the binder
 * is kept. That is the same arrangement {@code NwdBridge} already has with the
 * radio service, for the same reason.
 *
 * <h2>WHAT IS NOT KNOWN, AND A DRIVE IS THE ONLY WAY TO KNOW IT</h2>
 *
 * <p>The vendor wraps its own source change in an {@code AckHelper} that expects
 * an acknowledgement from the MCU and retries for three seconds. Handing over the
 * raw frame skips that bookkeeping entirely. Whether the MCU acts on an
 * unacknowledged frame from a package it has never heard of is not answerable
 * from a decompile. Every outcome here is reported by name so that one drive
 * settles it.
 */
final class CarnyxKernel {

    private static final String TAG = "CarnyxKernel";

    /** The package that owns the UART. */
    private static final String KERNEL_PACKAGE = "com.nwd.kernel";

    /**
     * The bind action, which is the service's own class name.
     *
     * <p>EXPORTED WITHOUT SAYING SO. The manifest entry declares an intent filter
     * on this action and no {@code android:permission}, and the manifest declares
     * no {@code targetSdkVersion} at all — so it defaults to
     * {@code minSdkVersion}, 19, far below the API 31 where {@code exported}
     * stopped defaulting to true for a component with a filter. Binding it is
     * ordinary third-party access, not a hole.
     */
    private static final String KERNEL_ACTION = "com.nwd.kernel.service.KernelService";

    /** The AIDL descriptor, which {@code Stub.onTransact} enforces. */
    private static final String DESCRIPTOR = "com.nwd.kernel.aidl.IKernelFeature";

    /** {@code void request(byte[])}. Read off {@code IKernelFeature$Stub}. */
    private static final int TRANSACTION_REQUEST = 1;

    /**
     * Hand the audio source back to Android.
     *
     * <p>THE SERVICE COMPUTES THE CHECKSUM, which is why the last byte is zero
     * and not arithmetic. {@code ProtocalUtil.writeDataToMCU} runs
     * {@code KernelProtocal.calCheckSumAndWriteEndOfData} on the caller's buffer
     * before it writes, so a caller cannot get it wrong and does not have to try.
     *
     * <pre>
     *     F0    header
     *     05    length
     *     01    protocol type 1, the ACTION family
     *     03    data type 3, CHANGE_SOURCE
     *     00    reserved
     *     00    sourceType: 0 is front
     *     00    sourceId:   0 is SOURCE_ANDROID (4 is SOURCE_RADIO)
     *     00    checksum, written by the service
     * </pre>
     */
    private static final byte[] SOURCE_TO_ANDROID = {
        (byte) 0xF0, 0x05, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00,
    };

    private static Context ctx;
    private static volatile IBinder kernel;
    private static boolean binding;

    private CarnyxKernel() {}

    private static final ServiceConnection CONN = new ServiceConnection() {
        @Override public void onServiceConnected(ComponentName name, IBinder service) {
            kernel = service;
            Log.i(TAG, "kernel service bound");
        }

        @Override public void onServiceDisconnected(ComponentName name) {
            // The process died or was updated. Android re-calls
            // onServiceConnected by itself when it comes back, so nothing is
            // re-bound here — dropping the stale binder is the whole job.
            kernel = null;
            Log.w(TAG, "kernel service disconnected");
        }
    };

    /**
     * Take the context and start binding. Safe to call more than once.
     *
     * @return a line for the diagnostics log, never null.
     */
    static synchronized String attach(Context context) {
        if (context == null) {
            return "kernel: no context";
        }
        if (ctx == null) {
            ctx = context.getApplicationContext();
        }
        if (kernel != null) {
            return "kernel: already bound";
        }
        if (binding) {
            return "kernel: bind already requested";
        }
        try {
            Intent i = new Intent(KERNEL_ACTION).setPackage(KERNEL_PACKAGE);
            // EXPLICIT BY PACKAGE, because API 21 forbids binding to an implicit
            // intent and would throw. The action alone is what the service
            // filters on; the package is what makes it addressable.
            boolean asked = ctx.bindService(i, CONN, Context.BIND_AUTO_CREATE);
            binding = asked;
            return asked
                    ? "kernel: binding to " + KERNEL_PACKAGE
                    : "kernel: bindService refused — no synchronous handback";
        } catch (Throwable t) {
            return "kernel: bind failed — " + t;
        }
    }

    /** Whether the synchronous path is available right now. */
    static boolean ready() {
        return kernel != null;
    }

    /**
     * Send the source back to Android, synchronously, and say what happened.
     *
     * <p>CALLED ON WHATEVER THREAD HEARD THE SLEEP, deliberately. The point of
     * this class is that the write reaches the UART before the call returns, and
     * hopping to another thread to make it would give back exactly the property
     * it exists to provide.
     *
     * <p>{@code transact} with flags 0 — NOT {@code FLAG_ONEWAY}. One-way would
     * return immediately and lose the race all over again, more quietly.
     *
     * @return one line for the diagnostics log, never null.
     */
    static String handBackSource() {
        IBinder b = kernel;
        if (b == null) {
            return "kernel: not bound, no synchronous handback";
        }
        if (!b.pingBinder()) {
            return "kernel: binder is dead";
        }
        Parcel data = Parcel.obtain();
        Parcel reply = Parcel.obtain();
        try {
            data.writeInterfaceToken(DESCRIPTOR);
            data.writeByteArray(SOURCE_TO_ANDROID);
            boolean ok = b.transact(TRANSACTION_REQUEST, data, reply, 0);
            if (!ok) {
                return "kernel: transact returned false";
            }
            // The service replies writeNoException(), so this throws only if the
            // far side threw. Reading it is what makes the call blocking in the
            // first place — a reply nobody reads is a reply nobody waits for.
            reply.readException();
            return "kernel: source→Android sent on the wire";
        } catch (Throwable t) {
            return "kernel: request failed — " + t;
        } finally {
            reply.recycle();
            data.recycle();
        }
    }
}
