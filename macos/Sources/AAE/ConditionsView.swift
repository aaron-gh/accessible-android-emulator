import SwiftUI

/// Sets what the device experiences: its battery, where it is, and text
/// messages and phone calls from the outside world.
struct ConditionsView: View {
    @EnvironmentObject var model: AppModel
    @State private var level = 100.0
    @State private var charging = true
    @State private var place = ""
    @State private var from = "5551234"
    @State private var message = ""
    @State private var number = "5551234"

    var body: some View {
        Form {
            Section {
                Slider(value: $level, in: 0...100, step: 5) {
                    Text("Battery level")
                }
                .accessibilityValue("\(Int(level)) percent")
                Toggle("Charging", isOn: $charging)
                Button("Set Battery") { model.setBattery(level: Int(level), charging: charging) }
            } header: {
                Text("Battery").accessibilityAddTraits(.isHeader)
            }
            Section {
                TextField("Place, address, or latitude and longitude", text: $place)
                    .onSubmit { model.setLocation(place) }
                Button("Set Location") { model.setLocation(place) }
            } header: {
                Text("Location").accessibilityAddTraits(.isHeader)
            }
            Section {
                TextField("From", text: $from)
                TextField("Message", text: $message, axis: .vertical)
                Button("Send Text Message") { model.sendTextMessage(from: from, text: message) }
                    .disabled(message.isEmpty)
            } header: {
                Text("Text Message").accessibilityAddTraits(.isHeader)
            }
            Section {
                TextField("Number", text: $number)
                HStack {
                    Button("Call the Device") { model.phoneCall(.ring, number: number) }
                    Button("Hang Up") { model.phoneCall(.hangUp, number: number) }
                    Button("Hold") { model.phoneCall(.hold, number: number) }
                    Button("Resume") { model.phoneCall(.resume, number: number) }
                }
                Text("When the device calls out, the number it calls can answer or be busy:")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                HStack {
                    Button("Answer the Device's Call") { model.phoneCall(.answer, number: number) }
                    Button("Be Busy") { model.phoneCall(.busy, number: number) }
                }
            } header: {
                Text("Phone Call").accessibilityAddTraits(.isHeader)
            }
        }
        .formStyle(.grouped)
        .disabled(!(model.selected?.running ?? false))
        .frame(minWidth: 520, minHeight: 560)
        .navigationTitle(model.selected.map { "Battery, Location and Phone: \($0.name)" } ?? "Battery, Location and Phone")
    }
}

/// The Device menu item that opens the Battery, Location and Phone window.
struct ConditionsMenuItem: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Battery, Location and Phone") { openWindow(id: "conditions") }
            .keyboardShortcut("b", modifiers: [.command, .option])
    }
}
