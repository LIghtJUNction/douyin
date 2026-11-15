# Metasec configuration fixture

`config-wire.json` contains public fake values serialized by the installed
40.8.0 APK's actual `ms.bd.c.u2.LIZ()` method in an isolated `app_process`.
The APK SHA-256 is
`a4e0682ff174d73dbc00d77622037090ee88f164fc0f9b62c324a1234e64ba57`.

The source is `tools/MetasecWireProbe.java`. Compile it together with
`tools/metasec-fixture/ms/bd/c/s4.java`, using Android SDK `android.jar`, then
convert both classes to DEX with d8. The probe takes the installed APK path.
Run the temporary DEX using a separate app_process and remove it afterwards.
It checks the APK digest before loading any APK classes.

The parent-loader `s4` replacement supplies only a fake SDK version, avoiding
the original holder's native decoder initialization. The serializer itself
must load from the APK. Since u2 is abstract, the probe instantiates its
concrete X.0mVm wrapper with an unused null builder reference. This calls the
u2 constructor; no SDK configuration/init command or app startup runs.

Cases cover constructor defaults, all 17 positions, Java trim (including
retaining NBSP), signed integer extremes, null/empty/whitespace keys,
non-string map values, and JSON escaping. Map inputs preserve the observed
Java iteration order; the Rust encoder does not reconstruct HashMap bucket
order from insertion order. `wireText` additionally permits byte comparison
for the configured case, including ASCII controls, slashes and U+2028/U+2029.

This fixture contains no account, device credential, signing key or real
license. It proves configuration encoding, not SDK initialization, request
signing, server acceptance or Android broadcasting.
