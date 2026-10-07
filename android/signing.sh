# Sourced by AAE's build scripts. Finds AAE's Android signing key, so the
# helper and eSpeak NG are signed the same way on every computer, and sets:
#   AAE_ANDROID_KEYSTORE      the key, by default ~/.android/aae.keystore
#   AAE_ANDROID_KEY_PASSWORD  its password, from the Keychain on a Mac
#   AAE_ANDROID_SIGNER        "aae" when the key can be used, or "debug" when
#                             builds fall back to this computer's debug key
export AAE_ANDROID_KEYSTORE="${AAE_ANDROID_KEYSTORE:-$HOME/.android/aae.keystore}"
if [[ -z "${AAE_ANDROID_KEY_PASSWORD:-}" && -f "$AAE_ANDROID_KEYSTORE" ]] && command -v security >/dev/null; then
    AAE_ANDROID_KEY_PASSWORD=$(security find-generic-password -a aae -s "AAE Android signing key" -w 2>/dev/null || true)
fi
export AAE_ANDROID_KEY_PASSWORD="${AAE_ANDROID_KEY_PASSWORD:-}"
if [[ -f "$AAE_ANDROID_KEYSTORE" && -n "$AAE_ANDROID_KEY_PASSWORD" ]]; then
    export AAE_ANDROID_SIGNER=aae
else
    export AAE_ANDROID_SIGNER=debug
    unset AAE_ANDROID_KEY_PASSWORD
fi
