// OsmAnd AIDL — INTEROP SCOPE, and a DELIBERATELY TRIMMED COPY. Read this
// before changing a single line, because the thing that makes it work is not
// visible in it.
//
// ── WHY EVERY SLOT IS HERE AND MOST OF THEM ARE EMPTY ────────────────────────
//
// A Binder transaction id is POSITIONAL: `aidl` numbers the methods it sees in
// declaration order from `FIRST_CALL_TRANSACTION`, and the number is all that
// crosses the wire. So an interface with the two methods we want and nothing
// else would compile, bind, and then call whatever OsmAnd happens to have at
// slot 0 and slot 1. The names do not travel; only the count and the order do.
//
// Upstream's own file is 958 lines and imports about a hundred parcelables,
// EVERY one of which `aidl` would need declared and `javac` would need
// implemented — for methods this app never calls. So the shape is kept and the
// payloads are not: 99 slots, in upstream's order, with real signatures on the
// two we invoke and `void reservedNN()` on the rest. A reserved slot generates
// a proxy method that is never called and an `onTransact` case that is never
// reached; what it does is hold the numbering.
//
// ── HOW THE NUMBERING IS CHECKED ─────────────────────────────────────────────
//
// `tools/check-osmand-aidl.sh` re-fetches upstream's interface and asserts that
// this file still has the same method COUNT and that our two are still at the
// same indices. A method inserted upstream ABOVE ours shifts every id below it
// and would otherwise be a silent wrong call on a head unit with no adb — which
// is the same class of failure `tools/check-jni.sh` exists for.
//
// This is not more fragile than vendoring the real file: that has identical
// positional ids. It is the same fragility, CHECKED rather than assumed.
//
// ── PROVENANCE ───────────────────────────────────────────────────────────────
//
// Reconstructed from osmandapp/OsmAnd, OsmAnd-api/src/net/osmand/aidlapi/,
// which is GPL-3.0-or-later; Carnyx is GPL-3.0-only, so the two are compatible.
// The package and interface names are load-bearing and cannot be renamed: the
// Binder DESCRIPTOR is the fully-qualified interface name, and OsmAnd rejects a
// transaction whose descriptor does not match.
//
// The service that serves this interface is declared in OsmAnd's own manifest as
//   <service android:name="net.osmand.aidl.OsmandAidlServiceV2" android:exported="true">
//     <intent-filter><action android:name="net.osmand.aidl.OsmandAidlServiceV2"/>
// with no permission and an `onBind` that returns the binder unconditionally.
package net.osmand.aidlapi;

import net.osmand.aidlapi.navigation.ANavigationUpdateParams;
import net.osmand.aidlapi.navigation.ANavigationVoiceRouterMessageParams;
import net.osmand.aidlapi.IOsmAndAidlCallback;

interface IOsmAndAidlInterface {

    void reserved00();  // upstream `addMapMarker`
    void reserved01();  // upstream `removeMapMarker`
    void reserved02();  // upstream `updateMapMarker`
    void reserved03();  // upstream `addMapWidget`
    void reserved04();  // upstream `removeMapWidget`
    void reserved05();  // upstream `updateMapWidget`
    void reserved06();  // upstream `addMapPoint`
    void reserved07();  // upstream `removeMapPoint`
    void reserved08();  // upstream `updateMapPoint`
    void reserved09();  // upstream `addMapLayer`
    void reserved10();  // upstream `removeMapLayer`
    void reserved11();  // upstream `updateMapLayer`
    void reserved12();  // upstream `importGpx`
    void reserved13();  // upstream `showGpx`
    void reserved14();  // upstream `hideGpx`
    void reserved15();  // upstream `getActiveGpx`
    void reserved16();  // upstream `setMapLocation`
    void reserved17();  // upstream `calculateRoute`
    void reserved18();  // upstream `refreshMap`
    void reserved19();  // upstream `addFavoriteGroup`
    void reserved20();  // upstream `removeFavoriteGroup`
    void reserved21();  // upstream `updateFavoriteGroup`
    void reserved22();  // upstream `addFavorite`
    void reserved23();  // upstream `removeFavorite`
    void reserved24();  // upstream `updateFavorite`
    void reserved25();  // upstream `startGpxRecording`
    void reserved26();  // upstream `stopGpxRecording`
    void reserved27();  // upstream `takePhotoNote`
    void reserved28();  // upstream `startVideoRecording`
    void reserved29();  // upstream `startAudioRecording`
    void reserved30();  // upstream `stopRecording`
    void reserved31();  // upstream `navigate`
    void reserved32();  // upstream `navigateGpx`
    void reserved33();  // upstream `removeGpx`
    void reserved34();  // upstream `showMapPoint`
    void reserved35();  // upstream `setNavDrawerItems`
    void reserved36();  // upstream `pauseNavigation`
    void reserved37();  // upstream `resumeNavigation`
    void reserved38();  // upstream `stopNavigation`
    void reserved39();  // upstream `muteNavigation`
    void reserved40();  // upstream `unmuteNavigation`
    void reserved41();  // upstream `search`
    void reserved42();  // upstream `navigateSearch`
    void reserved43();  // upstream `registerForUpdates`
    void reserved44();  // upstream `unregisterFromUpdates`
    void reserved45();  // upstream `setNavDrawerLogo`
    void reserved46();  // upstream `setEnabledIds`
    void reserved47();  // upstream `setDisabledIds`
    void reserved48();  // upstream `setEnabledPatterns`
    void reserved49();  // upstream `setDisabledPatterns`
    void reserved50();  // upstream `regWidgetVisibility`
    void reserved51();  // upstream `regWidgetAvailability`
    void reserved52();  // upstream `customizeOsmandSettings`
    void reserved53();  // upstream `getImportedGpx`
    void reserved54();  // upstream `getSqliteDbFiles`
    void reserved55();  // upstream `getActiveSqliteDbFiles`
    void reserved56();  // upstream `showSqliteDbFile`
    void reserved57();  // upstream `hideSqliteDbFile`
    void reserved58();  // upstream `setNavDrawerLogoWithParams`
    void reserved59();  // upstream `setNavDrawerFooterWithParams`
    void reserved60();  // upstream `restoreOsmand`
    void reserved61();  // upstream `changePluginState`
    void reserved62();  // upstream `registerForOsmandInitListener`
    void reserved63();  // upstream `getBitmapForGpx`
    void reserved64();  // upstream `copyFile`
    // slot 65 — upstream `registerForNavigationUpdates`. ONE OF THE TWO WE CALL.
    long registerForNavigationUpdates(in ANavigationUpdateParams params, IOsmAndAidlCallback callback);
    void reserved66();  // upstream `addContextMenuButtons`
    void reserved67();  // upstream `removeContextMenuButtons`
    void reserved68();  // upstream `updateContextMenuButtons`
    void reserved69();  // upstream `areOsmandSettingsCustomized`
    void reserved70();  // upstream `setCustomization`
    // slot 71 — upstream `registerForVoiceRouterMessages`. ONE OF THE TWO WE CALL.
    long registerForVoiceRouterMessages(in ANavigationVoiceRouterMessageParams params, IOsmAndAidlCallback callback);
    void reserved72();  // upstream `removeAllActiveMapMarkers`
    void reserved73();  // upstream `importProfile`
    void reserved74();  // upstream `executeQuickAction`
    void reserved75();  // upstream `getQuickActionsInfo`
    void reserved76();  // upstream `setLockState`
    void reserved77();  // upstream `registerForKeyEvents`
    void reserved78();  // upstream `getAppInfo`
    void reserved79();  // upstream `setMapMargins`
    void reserved80();  // upstream `exportProfile`
    void reserved81();  // upstream `isFragmentOpen`
    void reserved82();  // upstream `isMenuOpen`
    void reserved83();  // upstream `getPluginVersion`
    void reserved84();  // upstream `selectProfile`
    void reserved85();  // upstream `getProfiles`
    void reserved86();  // upstream `getBlockedRoads`
    void reserved87();  // upstream `addRoadBlock`
    void reserved88();  // upstream `removeRoadBlock`
    void reserved89();  // upstream `setLocation`
    void reserved90();  // upstream `exitApp`
    void reserved91();  // upstream `getText`
    void reserved92();  // upstream `reloadIndexes`
    void reserved93();  // upstream `setPreference`
    void reserved94();  // upstream `getPreference`
    void reserved95();  // upstream `registerForLogcatMessages`
    void reserved96();  // upstream `setZoomLimits`
    void reserved97();  // upstream `addWidgetGroup`
    void reserved98();  // upstream `removeWidgetGroup`
}
