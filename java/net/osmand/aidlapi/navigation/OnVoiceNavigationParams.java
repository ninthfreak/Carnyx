// OsmAnd AIDL — INTEROP SCOPE. See IOsmAndAidlInterface.aidl for provenance.
package net.osmand.aidlapi.navigation;

import android.os.Bundle;
import android.os.Parcel;

import net.osmand.aidlapi.AidlParams;

import java.util.ArrayList;
import java.util.List;

/**
 * One voice-router announcement, as WORDS — the only text OsmAnd's API gives an
 * outside app, and so the only place a street name can come from.
 *
 * <p>TWO LISTS, and they are not alternatives. {@code cmds} is the instruction
 * broken into its pieces as the router queued them; {@code played} is what the
 * voice engine actually said. `CarnyxNav` sends both across and lets Rust decide
 * which to show, because deciding here would put a display rule on the wrong
 * side of the wire — the rule this tree states for every Java class it has.
 */
public class OnVoiceNavigationParams extends AidlParams {

    private ArrayList<String> cmds = new ArrayList<>();
    private ArrayList<String> played = new ArrayList<>();

    public OnVoiceNavigationParams() {
    }

    public OnVoiceNavigationParams(Parcel in) {
        readFromParcel(in);
    }

    public static final Creator<OnVoiceNavigationParams> CREATOR = new Creator<OnVoiceNavigationParams>() {
        @Override public OnVoiceNavigationParams createFromParcel(Parcel in) { return new OnVoiceNavigationParams(in); }
        @Override public OnVoiceNavigationParams[] newArray(int size) { return new OnVoiceNavigationParams[size]; }
    };

    public List<String> getCommands() { return cmds; }

    public List<String> getPlayed() { return played; }

    @Override
    protected void readFromBundle(Bundle bundle) {
        cmds = bundle.getStringArrayList("cmds");
        if (cmds == null) {
            cmds = new ArrayList<>();
        }
        played = bundle.getStringArrayList("played");
        if (played == null) {
            played = new ArrayList<>();
        }
    }

    @Override
    public void writeToBundle(Bundle bundle) {
        bundle.putStringArrayList("cmds", cmds);
        bundle.putStringArrayList("played", played);
    }
}
