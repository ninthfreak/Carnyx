// OsmAnd AIDL — INTEROP SCOPE. See IOsmAndAidlInterface.aidl for provenance.
package net.osmand.aidlapi.map;

import android.os.Bundle;
import android.os.Parcel;

import net.osmand.aidlapi.AidlParams;

/**
 * A position, as {@code AppInfoParams} carries three of them.
 *
 * <h2>VENDORED FOR THE CLASSLOADER, NOT FOR THE DATA</h2>
 *
 * <p>Carnyx reads none of these — it has its own GPS fix — and this class exists
 * anyway, because of how Android 10 reads a Bundle. {@code BaseBundle.unparcel()}
 * is ALL-OR-NOTHING: the first getter on the bundle deserializes EVERY value in
 * it, whatever keys the caller actually asks for. {@code AppInfoParams} carries
 * three {@code putParcelable} values of this type, so the first
 * {@code getLong("arrivalTime")} tried to construct three of these through our
 * dex's classloader, found no class, and threw {@code BadParcelableException} —
 * once a second, into a logcat this unit cannot show, for the whole of a drive
 * with the permission gate open. The tell stayed gray and the poll never
 * answered while both subscriptions worked.
 *
 * <p>The belief that made the class skippable — "an unread parcelable is never
 * unmarshalled" — is TRUE from Android 13, where bundles hold lazy values that
 * deserialize per key. It was validated against the wrong Android. The unit is
 * Android 10, and the rule there is the opposite: EVERY parcelable type a
 * received bundle can carry must have a class the reader's classloader can
 * find. {@code tools/check-osmand-aidl.sh} now holds upstream's
 * {@code AppInfoParams.writeToBundle} to that rule against this tree.
 *
 * <p>The wire mirrors upstream exactly — an {@code AidlParams} bundle with keys
 * {@code latitude} and {@code longitude} — because unparcel calls THIS
 * reconstruction on THEIR bytes.
 */
public class ALatLon extends AidlParams {

    private double longitude;
    private double latitude;

    public ALatLon(double latitude, double longitude) {
        this.latitude = latitude;
        this.longitude = longitude;
    }

    protected ALatLon(Parcel in) {
        readFromParcel(in);
    }

    public static final Creator<ALatLon> CREATOR = new Creator<ALatLon>() {
        @Override public ALatLon createFromParcel(Parcel in) { return new ALatLon(in); }
        @Override public ALatLon[] newArray(int size) { return new ALatLon[size]; }
    };

    public double getLatitude() {
        return latitude;
    }

    public double getLongitude() {
        return longitude;
    }

    @Override
    public void writeToBundle(Bundle bundle) {
        bundle.putDouble("latitude", latitude);
        bundle.putDouble("longitude", longitude);
    }

    @Override
    protected void readFromBundle(Bundle bundle) {
        latitude = bundle.getDouble("latitude");
        longitude = bundle.getDouble("longitude");
    }
}
