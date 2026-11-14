# Public SDK serialization fixture

`client-data-40.8.json` was generated on Android from Douyin 40.8.0
(APK version 400801) using its `ClientDataTypeAdapter` and `X.1YtB` model.
The developer probe source is `tools/GuardWireProbe.java`.

All values are fixed public test data. `AQID` encodes bytes 1, 2, 3;
it is deliberately not an authentication signature. No real account,
ticket, key, attestation or device identifier is included.

The test compares decoded JSON fields and types. JSON property order is
not part of this comparison; signing uses the separately constructed
ticket/path/timestamp message, not these serialized JSON bytes.

To reproduce, compile the Java probe against Android API 36, convert it
to DEX using d8 (minimum API 26), and run it with root `app_process` on a
device with the pinned app build. Upload it under a unique temporary
path and remove it after execution. The probe only loads the installed
APK's pure model/serializer classes and uses public fixture values.
It rejects other builds instead of guessing their obfuscated symbols.

`token-proofs-40.8.json` covers six public legacy/combined proof examples.
Run the same probe with `--proofs` to reproduce it. The probe verifies the
installed `CombineTsSign` model's `SerializedName` annotations, constructs
that model with public strings/nulls, and reads its actual getters. Input
JSON is decoded by Android's `org.json`, not the app's Gson. An attempted
standalone Gson initialization required the app's ApplicationHolder/Keva
context and failed; this fixture does not claim a live SDK parser comparison.

The parse-JSON-first / legacy-value-to-TEE-only rule was separately traced
in the installed APK's `X.1Ysn.LJJIFFI` implementation. The Rust parser
intentionally rejects non-string proof fields, control characters and NULL
sentinels instead of reproducing Gson's coercions. These fixtures establish
field mapping and separation, not proof validity, account binding or mode
selection for a protected request.
