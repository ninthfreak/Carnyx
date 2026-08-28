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
 * <p>UNREAD IS NOT FREE, AND A DRIVE PROVED IT. An earlier version of this
 * paragraph said an unread parcelable is never unmarshalled and skipped
 * vendoring {@code ALatLon} on the strength of it. That is Android 13's rule —
 * lazy bundle values, deserialized per key. On Android 10, which this unit
 * runs, {@code unparcel()} is ALL-OR-NOTHING: the first getter deserializes
 * every value in the bundle, so the three unread {@code ALatLon}s threw
 * {@code BadParcelableException} on every poll and the feed was dead with the
 * permission gate wide open. The fields stay unread — but every parcelable
 * TYPE the bundle can carry must have a class in the dex, which is why
 * {@code map/ALatLon.java} exists and reads nothing.
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

    /**
     * Unix SECONDS at the destination, or 0 when not navigating — NOT millis,
     * despite the name and despite what this javadoc used to say.
     *
     * <p>Upstream {@code OsmandAidlApi.getAppInfo()} computes this as
     * {@code leftTime + System.currentTimeMillis() / 1000} — {@code leftTime}
     * is seconds ({@link #getLeftTime()}) and the millis term is divided down
     * to seconds before the add. A caller that treats the result as millis (as
     * this class's own javadoc did, and as {@code CarnyxNav.pollOnce} — the one
     * caller — used to) gets a value about three weeks after the epoch once
     * divided by 1000 a second time: a static, hours-wrong ETA is exactly what
     * that produces on the face, and exactly what a drive reported. See the
     * {@code * 1000L} at the call site.
     */
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
