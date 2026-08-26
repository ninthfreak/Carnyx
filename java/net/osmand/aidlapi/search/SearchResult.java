// OsmAnd AIDL — INTEROP SCOPE. See IOsmAndAidlInterface.aidl for provenance.
package net.osmand.aidlapi.search;

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
public class SearchResult extends AidlParams {

    public SearchResult() {
    }

    protected SearchResult(Parcel in) {
        readFromParcel(in);
    }

    public static final Creator<SearchResult> CREATOR = new Creator<SearchResult>() {
        @Override public SearchResult createFromParcel(Parcel in) { return new SearchResult(in); }
        @Override public SearchResult[] newArray(int size) { return new SearchResult[size]; }
    };
}
