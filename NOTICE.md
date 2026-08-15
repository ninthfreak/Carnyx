# Provenance

Carnyx is a Slint/Rust rebuild of the CarFM head-unit radio face and its
head-unit tuner integration. It is licensed **GPL-3.0-only**; the full text is
in `LICENSE`.

## Lineage

    VibeSDR (Stuart Carr / Stuey3D, GPL-3.0)
      └── CarFM (ninthfreak, GPL-3.0) — forked at fed883e4, 2026-07-14
            └── Carnyx (this repository, GPL-3.0)

Carnyx ports **CarFM's own work only**. Nothing that predates CarFM's fork of
VibeSDR has been carried across, and the boundary was established mechanically
rather than by inspection of filenames:

- The fork point is CarFM commit `01cd1da0` (2026-07-15), whose second parent
  `fed883e4` is VibeSDR's tip as it stood that day, confirmed present in
  Stuey3D/VibeSDR under the tag `alpha`.
- That boundary is exact: all 515 commits reachable from `fed883e4` are authored
  by Stuart Carr, and none of the 417 commits after it are. "Ancestor of
  `fed883e4`" and "authored by VibeSDR" are the same set.
- Every ported file was tested with `git merge-base --is-ancestor <adding-commit>
  fed883e4`, following renames — the Android package was renamed
  `com.vibesdr.app` → `com.ninthfreak.carfm`, so a path test alone would
  misreport.
- Renames are not the only hazard: a file added after the fork can still contain
  lifted content, which git records as an add. Every ported file was therefore
  also checked for verbatim overlap against an index of 38,331 substantial
  pre-fork source lines. Ported files scored 0–5%, and every non-zero hit was
  read: import statements and React Native boilerplate, no logic. A control group
  of known VibeSDR files scored 67–100% on the same measure.

Deliberately **not** ported: the DSP and SpyServer C++ trees, the `Vibe*` native
modules, the remote-SDR adapters (UberSDR, OpenWebRX, KiwiSDR, FM-DX), the
waterfall, and the advanced SDR screen. All are VibeSDR's.

## Station data

`assets/db/stations.sqlite` is derived from FCC public-domain licensing data,
snapshot 2026-07-16, and carries 20,733 rows. Its schema has a `logos` table
capable of holding third-party images; the shipped file has **zero** rows in it.
Nothing but FCC-derived data travels with this repository.

## Vendor interface code

`java/com/nwd/radio/service/` declares the NOWADA head unit's radio interface so
this app can bind to it. The `.aidl` files are interface declarations
reconstructed for interoperability; the two `.java` parcelables sit in the
vendor's own package namespace. CarFM's `docs/LICENSING.md` sets the standing
scope rule these were produced under — interoperability with our own device,
read-only, local, **no redistribution of decompiled code, modified APKs or
firmware** — and that rule travels with them.
