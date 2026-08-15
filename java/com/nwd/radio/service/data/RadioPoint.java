// VENDOR INTERFACE CODE — INTEROP SCOPE. See Frequency.java for the licensing
// note and the open provenance action that travels with both parcelables.
package com.nwd.radio.service.data;

import android.os.Parcel;
import android.os.Parcelable;

/**
 * Interop reconstruction of com.nwd.radio.service.data.RadioPoint — the band
 * plan for one band. Three ints, read in wire order.
 *
 * The first two were originally guessed as (max, min) and that guess was WRONG,
 * which the device settled on 2026-07-31. getRadioPoint() returned three bands:
 *
 *   FM   8750 / 10790 / 20     -> 87.5-107.9 MHz, 200 kHz steps
 *   AM    530 /  1710 / 10     -> 530-1710 kHz, 10 kHz steps
 *   (unused) 0 / 0 / 0
 *
 * Both real bands have the FIRST value below the second, and both match the
 * standard US allocation read that way round. So the wire order is LOW then
 * HIGH, and the fields are named for what they are rather than for the guess.
 * Renaming is safe: AIDL marshals parcelables by field ORDER, never by name.
 *
 * Units differ per band -- FM is in units of 10 kHz, AM in units of 1 kHz --
 * so these stay raw ints and the caller scales them.
 */
public class RadioPoint implements Parcelable {
    public int lo;
    public int hi;
    public int step;

    public RadioPoint() {}

    protected RadioPoint(Parcel in) {
        lo = in.readInt();
        hi = in.readInt();
        step = in.readInt();
    }

    @Override
    public void writeToParcel(Parcel d, int flags) {
        d.writeInt(lo);
        d.writeInt(hi);
        d.writeInt(step);
    }

    @Override
    public int describeContents() { return 0; }

    public static final Creator<RadioPoint> CREATOR = new Creator<RadioPoint>() {
        @Override public RadioPoint createFromParcel(Parcel in) { return new RadioPoint(in); }
        @Override public RadioPoint[] newArray(int size) { return new RadioPoint[size]; }
    };

    @Override
    public String toString() {
        return "RadioPoint{lo=" + lo + ", hi=" + hi + ", step=" + step + "}";
    }
}
