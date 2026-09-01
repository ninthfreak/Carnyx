package net.osmand.aidlapi.customization;

import android.os.Bundle;
import android.os.Parcel;

import net.osmand.aidlapi.AidlParams;

/**
 * One OsmAnd preference, by id, going out empty and coming back with a value.
 *
 * <p>INTEROP SCOPE, reconstructed from {@code osmandapp/OsmAnd},
 * {@code OsmAnd-api/src/net/osmand/aidlapi/customization/PreferenceParams.java}.
 * See {@code AidlParams} for the wire format and why reconstructing these is
 * safe: the whole parcel is one {@code Bundle}, so the fields are matched by
 * STRING KEY and not by position, and a key we spell wrong reads back null
 * instead of tearing the stream.
 *
 * <p>THE THREE KEYS ARE UPSTREAM'S AND ONE OF THEM DOES NOT MATCH ITS FIELD:
 * the field is {@code prefId} and the bundle key is {@code "preferenceId"}.
 * Spelling that key {@code "prefId"} would compile, bind, transact, and hand
 * OsmAnd a null preference id, which its {@code getPreference} answers with a
 * plain {@code false} — a working call that silently never finds anything.
 * {@code tools/check-osmand-aidl.sh} pins all three keys against upstream.
 *
 * <p>THE ONLY {@code inout} PARCELABLE IN THIS TREE, which is what makes it
 * different from the payloads beside it. Everything else Carnyx passes is
 * {@code in} — written on this side, read on OsmAnd's, done. This one is
 * written here, filled in there, and read back HERE, so the platform calls
 * {@code readFromParcel} on the very object that was sent. That method is
 * {@code final} in {@code AidlParams} and reads the bundle back through
 * {@link #readFromBundle}, so the round trip needs nothing of its own.
 */
public class PreferenceParams extends AidlParams {

    private String prefId;
    private String appModeKey;
    private String value;

    public PreferenceParams(String prefId) {
        this.prefId = prefId;
    }

    public PreferenceParams(Parcel in) {
        readFromParcel(in);
    }

    public static final Creator<PreferenceParams> CREATOR = new Creator<PreferenceParams>() {
        @Override
        public PreferenceParams createFromParcel(Parcel in) {
            return new PreferenceParams(in);
        }

        @Override
        public PreferenceParams[] newArray(int size) {
            return new PreferenceParams[size];
        }
    };

    public String getPrefId() {
        return prefId;
    }

    public String getAppModeKey() {
        return appModeKey;
    }

    public void setAppModeKey(String appModeKey) {
        this.appModeKey = appModeKey;
    }

    /** What OsmAnd wrote back, or null when it declined the read. */
    public String getValue() {
        return value;
    }

    public void setValue(String value) {
        this.value = value;
    }

    @Override
    public void writeToBundle(Bundle bundle) {
        bundle.putString("preferenceId", prefId);
        bundle.putString("appModeKey", appModeKey);
        bundle.putString("value", value);
    }

    @Override
    protected void readFromBundle(Bundle bundle) {
        prefId = bundle.getString("preferenceId");
        appModeKey = bundle.getString("appModeKey");
        value = bundle.getString("value");
    }
}
