// OsmAnd AIDL — INTEROP SCOPE. See IOsmAndAidlInterface.aidl for provenance.
package net.osmand.aidlapi.logcat;

import android.os.Parcel;

import net.osmand.aidlapi.AidlParams;

/**
 * A payload this app is DECLARED to receive and never reads.
 *
 * <p>IOsmAndAidlCallback names it on a slot we do not use, and aidl needs a type
 * for every case it generates. Deliberately EMPTY: the generated code unmarshals
 * one of these and drops it, so the inherited no-op bundle read is not a stub
 * standing in for something — it is the whole correct behaviour. Giving it
 * fields would be inventing a contract nothing checks.
 */
public class OnLogcatMessageParams extends AidlParams {

    public OnLogcatMessageParams() {
    }

    protected OnLogcatMessageParams(Parcel in) {
        readFromParcel(in);
    }

    public static final Creator<OnLogcatMessageParams> CREATOR = new Creator<OnLogcatMessageParams>() {
        @Override public OnLogcatMessageParams createFromParcel(Parcel in) { return new OnLogcatMessageParams(in); }
        @Override public OnLogcatMessageParams[] newArray(int size) { return new OnLogcatMessageParams[size]; }
    };
}
