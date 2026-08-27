// OsmAnd AIDL — INTEROP SCOPE. See IOsmAndAidlInterface.aidl for provenance.
package net.osmand.aidlapi;

import android.os.Bundle;
import android.os.Parcel;
import android.os.Parcelable;

/**
 * Interop reconstruction of {@code net.osmand.aidlapi.AidlParams}.
 *
 * <h2>The wire format, and why this one is safe where the vendor's was not</h2>
 *
 * <p>Every OsmAnd AIDL payload crosses as ONE {@link Bundle}: {@code
 * writeToParcel} is {@code dest.writeBundle(bundle)} and nothing else. So the
 * contract is a set of NAMED KEYS and their types — not, as with
 * {@code com.nwd.radio.service.data.Frequency} next door, a positional sequence
 * of field writes whose order had to be recovered from the service and is a
 * silent corruption if it is wrong.
 *
 * <p>That difference is worth stating because it is what makes reconstructing
 * these classes reasonable rather than reckless: a key we spell wrong reads back
 * as a zero, in one field, and a key upstream adds is ignored. Neither can
 * desynchronise the stream.
 *
 * <p>{@code writeToParcel} and {@code readFromParcel} are FINAL upstream and are
 * final here. Subclasses override the bundle halves only.
 */
public abstract class AidlParams implements Parcelable {

    @Override
    public final void writeToParcel(Parcel dest, int flags) {
        Bundle bundle = new Bundle();
        writeToBundle(bundle);
        dest.writeBundle(bundle);
    }

    public final void readFromParcel(Parcel in) {
        // THE CLASS LOADER IS THIS CLASS'S, as upstream has it — and it is
        // LOAD-BEARING, which an earlier version of this comment denied. It
        // said unmarshalling "for these payloads is never" needed: Android 13's
        // truth, where bundle values deserialize lazily per key. On this
        // unit's Android 10 the first getter unparcels EVERY value through
        // exactly this loader, so each parcelable TYPE any payload can carry
        // must be in our dex — see map/ALatLon.java for the drive that proved
        // it, and tools/check-osmand-aidl.sh for the check that now holds it.
        Bundle bundle = in.readBundle(getClass().getClassLoader());
        if (bundle != null) {
            readFromBundle(bundle);
        }
    }

    protected void writeToBundle(Bundle bundle) {
    }

    protected void readFromBundle(Bundle bundle) {
    }

    @Override
    public int describeContents() {
        return 0;
    }
}
