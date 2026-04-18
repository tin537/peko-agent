# remove_apps.mk — Subtract AOSP/LineageOS packages peko doesn't need.
#
# Frees ~800MB–1.2GB system partition space + ~400MB RAM at runtime.
# Included from peko_overlay.mk.
#
# KEEP list (load-bearing):
#   framework, services.jar, core-libart, wifi, telephony, bluetooth,
#   SettingsProvider, Permission controllers, logd, bootanimation.
#   Peko drives the UI itself so we keep framebuffer + input but drop
#   the launcher + systemui when in frameworkless mode.

# Apps / packages to NOT install (matched by module name)
PRODUCT_PACKAGES_REMOVE := \
    Calendar \
    CalendarProvider \
    Calculator \
    Camera \
    Camera2 \
    Contacts \
    ContactsProvider \
    DeskClock \
    Email \
    EmailExchange \
    ExactCalculator \
    Gallery \
    Gallery2 \
    LatinIME \
    LiveWallpapersPicker \
    Music \
    MusicFX \
    PackageInstaller \
    PartnerBookmarksProvider \
    PhotoTable \
    PicoTts \
    PrintRecommendationService \
    PrintSpooler \
    Stk \
    TeleService \
    Terminal \
    UserDictionaryProvider \
    VideoEditor \
    VpnDialogs \
    WallpaperBackup \
    WallpaperPicker \
    WallpaperCropper \
    \
    \
    LineageAudioService \
    LineageSetupWizard \
    Eleven \
    Jelly \
    LockClock \
    Snap \
    SnapdragonGallery \
    Updater \
    AudioFX \
    \
    \
    com.android.cts.ctsshim \
    com.android.cts.priv.ctsshim \
    EasterEgg \
    Traceur \
    WebViewGoogle \
    Browser2

# Keep SystemUI installed but override at runtime — init.peko.rc stops it
# when persist.peko.frameworkless=1 is set. Don't remove, because LOS
# first-boot still needs it to finish provisioning.

# Do NOT remove these even though tempting — things break:
#   SystemUI, Launcher3, Settings, SettingsProvider, Phone, Dialer,
#   TelephonyProvider, Telecom, TelephonyPackageServices,
#   MediaProviderGoogle, BlockedNumberProvider, SharedStorageBackup.
