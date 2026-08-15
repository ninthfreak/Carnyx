// VENDOR INTERFACE CODE — INTEROP SCOPE. See the note at the top of
// RadioFeature.aidl: interoperability RE of our own device's interface,
// read-only, local; do not redistribute decompiled code, modified APKs, or
// firmware.
//
// Reconstructed push-callback interface. NON-oneway (the service makes blocking
// callbacks and reads a reply — verified from the proxy). Method order = the real
// transaction codes (1..14). Do not reorder.
package com.nwd.radio.service;

import com.nwd.radio.service.data.Frequency;
import com.nwd.radio.service.data.RadioPoint;

interface RadioCallback {
    void notifyState(byte s);                                          // 1
    void notifyCurrentFrequency(byte band, int freq, String ps, int arg); // 2  freq + PS name
    void notifyNearOn(boolean on);                                     // 3
    void notifyStereo(boolean on);                                     // 4
    void notifyStereoOn(boolean on);                                   // 5
    void notifyRDSStateChange();                                       // 6
    void notifyCurrentPTYType(byte pty);                               // 7
    void notifyPrefabFrequency(in Frequency[] arr);                    // 8
    void notifyPrefabPTYType(byte pty);                                // 9
    void notifyRadioPoint(in RadioPoint[] arr);                        // 10
    void notifyCurrentIsTA(boolean ta);                                // 11
    void notifyRdsShowState(boolean on);                               // 12
    void notifyRtMessage(String rt);                                   // 13 RadioText push
    void notifyRadioScanState(int state);                              // 14
}
