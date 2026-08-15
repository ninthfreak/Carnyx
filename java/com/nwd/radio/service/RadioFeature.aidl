// VENDOR INTERFACE CODE — INTEROP SCOPE, CARRIED OVER FROM CarFM.
//
// CarFM's docs/LICENSING.md flags this file, RadioCallback.aidl and the two
// data/*.aidl + data/*.java parcelables as vendor interface code, and puts them
// under the standing scope rule from docs/BUILTIN-TUNER-FINDINGS.md:
//
//   "Interoperability RE of our own device's interface, read-only, local. Do not
//    redistribute decompiled code, modified APKs, or firmware."
//
// These are interface DECLARATIONS reconstructed by hand for interoperability —
// a description of the interface, not vendor source. The rule travels with the
// files: it is why they are here and it is the condition on keeping them here.
// LICENSING.md rates the .java parcelables as the more substantive pair, and
// leaves an OPEN action against them (confirm each is hand-written from the
// interface rather than decompiler output, and document that). That action is
// still open; it did not close by being copied into this repository.
//
// Reconstructed from the on-device com.nwd.radio.service interface for interop.
// METHOD ORDER IS LOAD-BEARING: AIDL assigns transaction codes in declaration
// order (1..N), and these must match the service's real codes (verified from the
// service's TRANSACTION_* constants). Do not reorder.
package com.nwd.radio.service;

import com.nwd.radio.service.RadioCallback;
import com.nwd.radio.service.data.Frequency;
import com.nwd.radio.service.data.RadioPoint;

interface RadioFeature {
    void setCurrentFrequency(int freq, byte band, int flag);   // 1  tune
    Frequency getCurrentFrequency();                           // 2
    void seek(boolean up);                                     // 3
    void search(boolean up);                                   // 4  full scan
    void changeBand();                                         // 5
    void AMS();                                                // 6  auto memory store
    void INTRO();                                              // 7  intro scan
    void setNearOn(boolean on);                                // 8  local/DX
    boolean isNearOn();                                        // 9
    boolean isHasStrero();                                     // 10
    void setStreroOn(boolean on);                              // 11
    boolean isStreroOn();                                      // 12
    void setRadioBackServiceOn(boolean on);                    // 13 background audio
    boolean isRadioBackServiceOn();                            // 14
    void setRDSState(byte which, boolean on);                  // 15
    boolean getRDSState(int which);                            // 16
    void setPTYType(byte pty);                                 // 17
    byte getPTYType();                                         // 18
    byte getPrefabPTYType();                                   // 19
    void saveCurrentFrequency(byte index);                     // 20 save preset
    Frequency[] getPrefabFrequency();                          // 21 presets
    RadioPoint[] getRadioPoint();                              // 22 band plan (min/max/step)
    byte getRadioState();                                      // 23
    void registCallback(RadioCallback cb);                     // 24
    void unRegistCallback(RadioCallback cb);                   // 25
    void prefeb(boolean b);                                    // 26
    void sendRadioCommand(byte a, byte b);                     // 27 raw MCU command escape hatch
    String getRtMessage();                                     // 28 RadioText
    int getRadioType();                                        // 29
    int getCurrentScanState();                                 // 30
}
