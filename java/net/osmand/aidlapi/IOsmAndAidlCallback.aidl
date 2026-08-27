// OsmAnd AIDL — INTEROP SCOPE. See IOsmAndAidlInterface.aidl for the provenance
// and the transaction-id rule; this file is that rule's other half.
//
// VERBATIM IN SHAPE, unlike the interface beside it, and for the opposite
// reason: we IMPLEMENT this one. OsmAnd holds the proxy and dials the slot, so
// our Stub has to answer at the same index upstream dials — and there are nine
// methods here rather than ninety-nine, so keeping all of them costs almost
// nothing. `updateNavigationInfo` is slot 4 and `onVoiceRouterNotify` is slot 6;
// `tools/check-osmand-aidl.sh` asserts both.
//
// THE PARCELABLES WE DO NOT READ ARE STILL DECLARED, because `aidl` needs a type
// for every parameter it generates a case for. Their `.java` reconstructions are
// empty `AidlParams` subclasses: they are unmarshalled by the generated code and
// then dropped, and an empty bundle read is exactly right for a value nothing
// looks at.
package net.osmand.aidlapi;

import net.osmand.aidlapi.search.SearchResult;
import net.osmand.aidlapi.gpx.AGpxBitmap;
import net.osmand.aidlapi.navigation.ADirectionInfo;
import net.osmand.aidlapi.navigation.OnVoiceNavigationParams;
import net.osmand.aidlapi.logcat.OnLogcatMessageParams;
// THE ONE LINE UPSTREAM DOES NOT HAVE. Their build resolves the bare `KeyEvent`
// below against the SDK's preprocessed framework.aidl (`-p`), which our aidl
// run does not pass — it resolves through `-I` and this import, against the
// declaration at java/android/view/KeyEvent.aidl. Same type, same slot, stated
// where our pipeline can read it; an import changes no transaction id.
import android.view.KeyEvent;

interface IOsmAndAidlCallback {

    void onSearchComplete(in List<SearchResult> resultSet);

    void onUpdate();

    void onAppInitialized();

    void onGpxBitmapCreated(in AGpxBitmap bitmap);

    /** Slot 4 — the one this app is here for. */
    void updateNavigationInfo(in ADirectionInfo directionInfo);

    void onContextMenuButtonClicked(in int buttonId, String pointId, String layerId);

    /** Slot 6 — the spoken instruction, which is the only place a STREET NAME
     *  reaches an API client. `ADirectionInfo` carries no text at all. */
    void onVoiceRouterNotify(in OnVoiceNavigationParams params);

    void onKeyEvent(in KeyEvent params);

    void onLogcatMessage(in OnLogcatMessageParams params);
}
