// OsmAnd AIDL — INTEROP SCOPE. See net/osmand/aidlapi/IOsmAndAidlInterface.aidl
// for the provenance rule this tree follows.
//
// ── A FRAMEWORK PARCELABLE, DECLARED SO OUR OWN AIDL RUN CAN SEE IT ──────────
//
// `IOsmAndAidlCallback.onKeyEvent` takes Android's own `android.view.KeyEvent`.
// Upstream compiles that file WITHOUT importing it, because Gradle's AIDL step
// passes the SDK's preprocessed declarations (`-p …/platforms/android-NN/
// framework.aidl`, whose line for this type is literally
// `parcelable android.view.KeyEvent;`) and the tool resolves the bare name
// against them. Our build.rs runs `aidl -I<java root>` with no `-p`, so the
// first device build after the callback was vendored died right here:
//
//   ERROR: … Couldn't find import for class KeyEvent. Searched here:
//    - …/Carnyx/java/
//
// This file is the same declaration framework.aidl carries, placed where our
// include path can find it — the mechanism every net.osmand and com.nwd import
// in this tree already resolves through, proven on the unit for weeks.
//
// NO `.java` BESIDE IT, EVER. The other parcelables here carry hand-written
// reconstructions because OsmAnd's classes are not in the APK; `KeyEvent` IS in
// `android.jar` and on every device, so javac and the runtime both use the real
// one. A reconstruction would shadow the framework class and break the
// unmarshalling it exists to do.
package android.view;

parcelable KeyEvent;
