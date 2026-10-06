import AppKit
import SwiftUI

/// One device in a window of its own, so several devices can each have a
/// window, with its own keyboard focus. While its window is in front, the
/// device is the selected one, so the Device menu and its shortcuts act on
/// it, and device mode started here stays in this window.
struct DeviceWindowView: View {
    @EnvironmentObject var model: AppModel
    let id: String

    var body: some View {
        Group {
            if model.inDeviceMode && model.deviceModeHost == id {
                DeviceModeView()
            } else if let device {
                controls(device)
            } else {
                Text("This device no longer exists.")
                    .padding()
            }
        }
        .frame(minWidth: 460, minHeight: 260)
        .navigationTitle(device?.name ?? "Device")
        .background(WindowFocusReporter { isKey in
            if isKey {
                model.keyDeviceWindow = id
                if model.selection != id { model.selection = id }
            } else if model.keyDeviceWindow == id {
                model.keyDeviceWindow = nil
            }
        })
    }

    private var device: DeviceInfo? {
        model.devices.first { $0.id == id }
    }

    private func controls(_ device: DeviceInfo) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(device.name)
                .font(.title2)
                .accessibilityAddTraits(.isHeader)
            Text("\(device.android), \(device.kind), \(model.busy[device.id]?.lowercased() ?? (device.running ? "running" : "stopped")).")
            HStack {
                if model.busy[device.id] != nil {
                    ProgressView().controlSize(.small).accessibilityLabel(model.busy[device.id] ?? "")
                } else if device.running {
                    Button("Use Android Keyboard") { model.enterDeviceMode(host: id) }
                    Button("Use Gestures") { model.enterDeviceMode(gestures: true, host: id) }
                    Button("Stop") { model.stop(id) }
                } else {
                    Button("Start") { model.start(id) }
                }
            }
            if device.running, model.busy[device.id] == nil {
                HStack {
                    Button("Back") { model.press("back") }
                    Button("Home") { model.press("home") }
                    Button("Recent Apps") { model.press("recents") }
                    Button("Notifications") { model.showNotifications() }
                }
                Slider(
                    value: Binding(get: { Double(device.volume) }, set: { model.setVolume(Float($0)) }),
                    in: 0...1,
                    step: 0.05
                ) {
                    Text("Volume of \(device.name)")
                }
                .accessibilityValue("\(Int((device.volume * 100).rounded())) percent")
                .frame(maxWidth: 360)
            }
            Spacer()
        }
        .padding()
    }
}

/// Tells a SwiftUI view when its window becomes, or stops being, the key window.
struct WindowFocusReporter: NSViewRepresentable {
    let changed: (Bool) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = FocusView()
        view.changed = changed
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        (view as? FocusView)?.changed = changed
    }

    final class FocusView: NSView {
        var changed: ((Bool) -> Void)?
        private var tokens: [NSObjectProtocol] = []

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            tokens.forEach(NotificationCenter.default.removeObserver)
            tokens = []
            guard let window else { return }
            let center = NotificationCenter.default
            tokens = [
                center.addObserver(forName: NSWindow.didBecomeKeyNotification, object: window, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.changed?(true) }
                },
                center.addObserver(forName: NSWindow.didResignKeyNotification, object: window, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.changed?(false) }
                },
            ]
            if window.isKeyWindow { changed?(true) }
        }
    }
}
