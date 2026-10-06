import Combine
import Sparkle
import SwiftUI

/// Updates AAE itself, through Sparkle, as most Mac apps update.
///
/// Sparkle's own process downloads the new version, checks it was signed
/// with AAE's key, and offers Install and Relaunch, all in standard AppKit
/// windows that VoiceOver reads like any other. On first launch Sparkle asks
/// once whether to check automatically. Automatic checks stay silent unless
/// there's an update; Check for Updates always answers, even "you're up to
/// date", because the user asked. The same approach as AVM's updater.
@MainActor
final class Updater: ObservableObject {
    private let controller = SPUStandardUpdaterController(
        startingUpdater: true,
        updaterDelegate: nil,
        userDriverDelegate: nil
    )

    /// Whether a check can start now; false while one is already going.
    @Published var canCheckForUpdates = false

    init() {
        controller.updater.publisher(for: \.canCheckForUpdates)
            .assign(to: &$canCheckForUpdates)
    }

    func checkForUpdates() {
        controller.checkForUpdates(nil)
    }
}

/// The app menu item that checks for updates.
struct CheckForUpdatesMenuItem: View {
    @ObservedObject var updater: Updater

    var body: some View {
        Button("Check for Updates…") { updater.checkForUpdates() }
            .disabled(!updater.canCheckForUpdates)
    }
}
