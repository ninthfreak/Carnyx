// OsmAnd AIDL — INTEROP SCOPE. See IOsmAndAidlInterface.aidl for provenance.
package net.osmand.aidlapi.navigation;

import android.os.Bundle;
import android.os.Parcel;

import net.osmand.aidlapi.AidlParams;

/**
 * What one navigation update carries — and it is less than it looks.
 *
 * <p>THREE INTS AND NO WORDS. There is no street name here, no exit number, no
 * ETA and no distance to the destination; `CarnyxNav` takes the spoken
 * instruction from {@link OnVoiceNavigationParams} for the text, because this is
 * the whole of the structured half.
 *
 * <h2>What the values mean, read out of OsmAnd's own sender</h2>
 *
 * <p>{@code OsmandAidlApi.registerForNavigationUpdates} builds this as
 * {@code new ADirectionInfo(-1, -1, false)} and then:
 * <ul>
 *   <li>deviated from the route → {@code turnType = TurnType.OFFR} (12) and
 *       {@code distanceTo} = the DEVIATION, not a distance to any turn;
 *   <li>otherwise, when there is a next direction at all →
 *       {@code turnType = ndi.directionInfo.getTurnType().getValue()}, a bare
 *       {@code TurnType} constant in 1..14, and {@code distanceTo} = metres to
 *       it;
 *   <li>otherwise the {@code -1, -1} it was built with, which is "navigating,
 *       nothing to say" and NOT an error.
 * </ul>
 *
 * <p>{@code isLeftSide} is set by the constructor and never assigned again on
 * that path, so it arrives false whatever the country's driving side is. It is
 * carried because it is on the wire; nothing reads it.
 */
public class ADirectionInfo extends AidlParams {

    private int distanceTo;
    private int turnType;
    private boolean isLeftSide;

    public ADirectionInfo(int distanceTo, int turnType, boolean isLeftSide) {
        this.distanceTo = distanceTo;
        this.turnType = turnType;
        this.isLeftSide = isLeftSide;
    }

    protected ADirectionInfo(Parcel in) {
        readFromParcel(in);
    }

    public static final Creator<ADirectionInfo> CREATOR = new Creator<ADirectionInfo>() {
        @Override public ADirectionInfo createFromParcel(Parcel in) { return new ADirectionInfo(in); }
        @Override public ADirectionInfo[] newArray(int size) { return new ADirectionInfo[size]; }
    };

    public int getDistanceTo() { return distanceTo; }

    public int getTurnType() { return turnType; }

    public boolean isLeftSide() { return isLeftSide; }

    @Override
    protected void readFromBundle(Bundle bundle) {
        distanceTo = bundle.getInt("distanceTo");
        turnType = bundle.getInt("turnType");
        isLeftSide = bundle.getBoolean("isLeftSide");
    }

    @Override
    public void writeToBundle(Bundle bundle) {
        bundle.putInt("distanceTo", distanceTo);
        bundle.putInt("turnType", turnType);
        bundle.putBoolean("isLeftSide", isLeftSide);
    }
}
