// OsmAnd AIDL — INTEROP SCOPE. See IOsmAndAidlInterface.aidl for provenance.
package net.osmand.aidlapi.info;

import android.os.Bundle;
import android.os.Parcel;

import net.osmand.aidlapi.AidlParams;

/**
 * What `getAppInfo` answers — the POLL half of the navigation feed.
 *
 * <h2>Why there are two channels at all</h2>
 *
 * <p>The push callback ({@code ADirectionInfo}) is three integers and no words.
 * The ETA, the distance left, the STREET NAME and the turn after next exist only
 * here, and only if something asks. That is why `CarnyxNav` runs a timer as well
 * as holding a callback.
 *
 * <h2>A DELIBERATELY PARTIAL RECONSTRUCTION</h2>
 *
 * <p>Upstream declares twelve fields; this reads six. The three {@code ALatLon}
 * positions, {@code versionsInfo}, {@code osmAndVersion}, {@code releaseDate} and
 * {@code routingData} are left in the bundle unread — Carnyx has its own GPS fix
 * and no use for the rest.
 *
 * <p>THAT IS SAFE ONLY BECAUSE THE WIRE IS A BUNDLE. {@code AidlParams} writes
 * one {@code writeBundle} and the keys are named, so an unread key costs nothing
 * and an unread PARCELABLE is never unmarshalled — which is what lets this class
 * skip {@code ALatLon} without vendoring it. A positional format could not be
 * read partially at all.
 *
 * <h2>The turnInfo bundle</h2>
 *
 * <p>A nested {@link Bundle}, and its keys are built by string concatenation in
 * OsmAnd's {@code ExternalApiHelper.updateTurnInfo(prefix, …)} — so the caller
 * reads them by name and the prefixes are part of the contract. See
 * {@code CarnyxNav.readTurn} for the one that is not what it looks like.
 */
public class AppInfoParams extends AidlParams {

    private long arrivalTime;
    private int leftTime;
    private int leftDistance;
    private boolean mapVisible;
    private Bundle turnInfo;

    public AppInfoParams() {
    }

    protected AppInfoParams(Parcel in) {
        readFromParcel(in);
    }

    public static final Creator<AppInfoParams> CREATOR = new Creator<AppInfoParams>() {
        @Override public AppInfoParams createFromParcel(Parcel in) { return new AppInfoParams(in); }
        @Override public AppInfoParams[] newArray(int size) { return new AppInfoParams[size]; }
    };

    /** Unix millis at the destination, or 0 when not navigating. */
    public long getArrivalTime() { return arrivalTime; }

    /** Seconds left, or 0. */
    public int getLeftTime() { return leftTime; }

    /** Metres left, or 0. */
    public int getLeftDistance() { return leftDistance; }

    /** True when OsmAnd's own map is in front — the driver can already see it. */
    public boolean isMapVisible() { return mapVisible; }

    /** Null when there is no route. See the class note. */
    public Bundle getTurnInfo() { return turnInfo; }

    @Override
    protected void readFromBundle(Bundle bundle) {
        arrivalTime = bundle.getLong("arrivalTime");
        leftTime = bundle.getInt("leftTime");
        leftDistance = bundle.getInt("leftDistance");
        mapVisible = bundle.getBoolean("mapVisible");
        turnInfo = bundle.getBundle("turnInfo");
    }

    @Override
    public void writeToBundle(Bundle bundle) {
        // WRITTEN FOR SYMMETRY AND NEVER SENT. This type only ever travels
        // OsmAnd → here; the generated stub needs a writer to compile.
        bundle.putLong("arrivalTime", arrivalTime);
        bundle.putInt("leftTime", leftTime);
        bundle.putInt("leftDistance", leftDistance);
        bundle.putBoolean("mapVisible", mapVisible);
        bundle.putBundle("turnInfo", turnInfo);
    }
}
