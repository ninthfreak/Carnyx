// OsmAnd AIDL — INTEROP SCOPE. See IOsmAndAidlInterface.aidl for provenance.
package net.osmand.aidlapi.navigation;

import android.os.Bundle;
import android.os.Parcel;

import net.osmand.aidlapi.AidlParams;

/**
 * The subscription request for registerForNavigationUpdates: a callback id and a flag.
 *
 * <p>Both bundle keys are upstream's, and the pair is shared with the other
 * register* params class beside it — the id is the handle to unsubscribe with
 * and is ignored on the way in, and the flag is what turns the stream on and
 * off. Unsubscribing is the same call with the id OsmAnd returned and the flag
 * set false.
 */
public class ANavigationUpdateParams extends AidlParams {

    private boolean subscribeToUpdates = true;
    private long callbackId = -1L;

    public ANavigationUpdateParams() {
    }

    protected ANavigationUpdateParams(Parcel in) {
        readFromParcel(in);
    }

    public static final Creator<ANavigationUpdateParams> CREATOR = new Creator<ANavigationUpdateParams>() {
        @Override public ANavigationUpdateParams createFromParcel(Parcel in) { return new ANavigationUpdateParams(in); }
        @Override public ANavigationUpdateParams[] newArray(int size) { return new ANavigationUpdateParams[size]; }
    };

    public long getCallbackId() { return callbackId; }

    public void setCallbackId(long callbackId) { this.callbackId = callbackId; }

    public void setSubscribeToUpdates(boolean subscribeToUpdates) {
        this.subscribeToUpdates = subscribeToUpdates;
    }

    @Override
    protected void readFromBundle(Bundle bundle) {
        callbackId = bundle.getLong("callbackId");
        subscribeToUpdates = bundle.getBoolean("subscribeToUpdates");
    }

    @Override
    public void writeToBundle(Bundle bundle) {
        bundle.putLong("callbackId", callbackId);
        bundle.putBoolean("subscribeToUpdates", subscribeToUpdates);
    }
}
