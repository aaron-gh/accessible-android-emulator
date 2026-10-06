import SwiftUI

/// The selected device's accessibility services. Screen readers come first,
/// as one choice, since only one runs at a time; every other service has a
/// switch. What's turned on stays on, and what's turned off stays off.
struct ServicesView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(model.servicesDevice.map { "Accessibility Services on \($0)" } ?? "Accessibility Services")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            if model.services.isEmpty {
                Text(emptyText).foregroundStyle(.secondary)
            } else {
                Form {
                    Section {
                        Picker("Screen reader", selection: screenReader) {
                            Text("None").tag("")
                            ForEach(screenReaders, id: \.component) { service in
                                Text(service.label).tag(service.component)
                            }
                        }
                        .pickerStyle(.radioGroup)
                    } header: {
                        Text("Screen Reader").accessibilityAddTraits(.isHeader)
                    }
                    Section {
                        ForEach(others, id: \.component) { service in
                            VStack(alignment: .leading, spacing: 2) {
                                Toggle(service.label, isOn: Binding(
                                    get: { service.on },
                                    set: { model.setService(service, on: $0) }
                                ))
                                if !service.description.isEmpty {
                                    Text(service.description)
                                        .font(.callout)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(3)
                                }
                            }
                        }
                    } header: {
                        Text("Other Services").accessibilityAddTraits(.isHeader)
                    }
                }
                .formStyle(.grouped)
            }
            Button("Refresh") { model.loadServices() }
                .keyboardShortcut("r")
        }
        .padding()
        .frame(minWidth: 540, minHeight: 440)
        .onAppear { model.loadServices() }
        .onChange(of: model.selection) { _ in model.loadServices() }
    }

    private var screenReaders: [ServiceInfo] { model.services.filter(\.screenReader) }
    private var others: [ServiceInfo] { model.services.filter { !$0.screenReader } }

    /// The screen reader that's on; choosing another switches to it, and None
    /// turns the one that's on off.
    private var screenReader: Binding<String> {
        Binding(
            get: { screenReaders.first(where: \.on)?.component ?? "" },
            set: { component in
                if component.isEmpty {
                    if let current = screenReaders.first(where: \.on) {
                        model.setService(current, on: false)
                    }
                } else if let service = screenReaders.first(where: { $0.component == component }) {
                    model.setService(service, on: true)
                }
            }
        )
    }

    private var emptyText: String {
        if !(model.selected?.running ?? false) { return "Start the device to see its accessibility services." }
        return model.loadingServices ? "Reading…" : "No accessibility services are installed."
    }
}

/// The Device menu item that opens the Accessibility Services window.
struct ServicesMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Accessibility Services") { openWindow(id: "services") }
            .keyboardShortcut("u", modifiers: [.command, .option])
    }
}
