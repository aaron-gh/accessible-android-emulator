import SwiftUI

/// A stopped device's advanced hardware: memory, processor cores, storage,
/// and the screen's size and density, from its next start.
struct HardwareView: View {
    @EnvironmentObject var model: AppModel
    let device: DeviceInfo
    @State private var memory = ""
    @State private var cores = ""
    @State private var storage = ""
    @State private var width = ""
    @State private var height = ""
    @State private var density = ""
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Hardware of \(device.name)").font(.headline).accessibilityAddTraits(.isHeader)
            Form {
                TextField("Memory, in megabytes", text: $memory)
                TextField("Processor cores", text: $cores)
                TextField("Storage, in megabytes", text: $storage)
                TextField("Screen width, in pixels", text: $width)
                TextField("Screen height, in pixels", text: $height)
                TextField("Screen density, in dots per inch", text: $density)
            }
            Text("Applies at next start. Reducing storage needs a wipe.")
                .font(.callout)
                .foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save", action: save)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding()
        .frame(width: 420)
        .onAppear(perform: load)
    }

    private func load() {
        guard let hardware = model.hardware(of: device) else { return }
        memory = String(hardware.memoryMb)
        cores = String(hardware.cores)
        storage = String(hardware.storageMb)
        width = String(hardware.width)
        height = String(hardware.height)
        density = String(hardware.density)
    }

    private func save() {
        let numbers = [memory, cores, storage, width, height, density].map {
            UInt32($0.trimmingCharacters(in: .whitespaces))
        }
        guard numbers.allSatisfy({ $0 != nil }) else {
            model.announce("Each one takes a whole number.", tone: .failure)
            return
        }
        let n = numbers.map { $0! }
        let hardware = HardwareInfo(memoryMb: n[0], cores: n[1], storageMb: n[2], width: n[3], height: n[4], density: n[5])
        if model.setHardware(of: device, to: hardware) {
            dismiss()
        }
    }
}
