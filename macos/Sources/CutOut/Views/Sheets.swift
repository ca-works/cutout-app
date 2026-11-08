import SwiftUI

struct ModelSetupSheet: View {
    @Environment(EditorModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var manager = ModelManager.shared
    @AppStorage("bestModel") private var bestModel = ModelSpec.birefnetGeneral.id
    @AppStorage("useCoreML") private var useCoreML = true
    @State private var refresh = 0

    private enum CoreMLState {
        case ready(CoreMLBiRefNet.Source), notInstalled, disabled, unsupported, failed(String)
    }

    private var coreMLState: CoreMLState {
        _ = refresh
        if !CoreMLBiRefNet.isSupportedHardware { return .unsupported }
        if let f = manager.coreMLFailure { return .failed(f) }
        guard let src = CoreMLBiRefNet.locate() else { return .notInstalled }
        return useCoreML ? .ready(src) : .disabled
    }

    /// Which backend Best Quality will use next.
    private var nextBackend: String {
        if case .ready = coreMLState, bestModel != ModelSpec.birefnetLite.id { return "Core ML · \(CoreMLBiRefNet.unitsLabel)" }
        if let s = manager.installedBestSpec { return "ONNX Runtime · CPU (\(s.title))" }
        return "None installed — Fast mode (Apple Vision) is available"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Image(systemName: "wand.and.sparkles").font(.largeTitle).foregroundStyle(.tint)
                VStack(alignment: .leading) {
                    Text("Best-Quality Background Removal").font(.title3.weight(.semibold))
                    Text("BiRefNet runs entirely on this Mac. Once a model is present CutOut works offline.")
                        .font(.callout).foregroundStyle(.secondary)
                }
            }
            LabeledContent("Best Quality uses") {
                Text(nextBackend).fontWeight(.medium)
            }
            if let last = manager.activeBackend {
                LabeledContent("Last removal ran on") { Text(last).foregroundStyle(.secondary) }
            }

            coreMLCard

            Text("ONNX models (CPU — automatic fallback, ~14 s per image)").font(.headline).padding(.top, 4)
            ForEach(ModelSpec.all) { spec in onnxRow(spec) }

            if let e = manager.lastError {
                Label(e, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red).font(.callout)
            }
            Text("Downloads come from huggingface.co at pinned revisions and are SHA-256 verified. No images are ever uploaded.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("Use Fast Mode Now") {
                    dismiss()
                    model.removeBackground(.fast)
                }
                .help("Apple Vision subject lifting — built in, no download")
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
                if manager.hasBestModel, !model.isEmpty {
                    Button("Remove Background") {
                        dismiss()
                        model.removeBackground(.best)
                    }
                    .buttonStyle(.borderedProminent)
                }
            }
        }
        .padding(22)
        .frame(width: 580)
    }

    @ViewBuilder private var coreMLCard: some View {
        HStack(alignment: .top, spacing: 12) {
            switch coreMLState {
            case .ready: Image(systemName: "bolt.circle.fill").foregroundStyle(.green).font(.title3)
            case .failed: Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange).font(.title3)
            default: Image(systemName: "bolt.circle").foregroundStyle(.secondary).font(.title3)
            }
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text("BiRefNet · Core ML (fastest)").font(.headline)
                    Text("≈2–3 s per image").foregroundStyle(.secondary)
                }
                switch coreMLState {
                case .ready(let src):
                    Text("Ready — \(src.label). Runs on the \(CoreMLBiRefNet.unitsLabel); compiled on first use and cached.")
                        .font(.callout).foregroundStyle(.secondary)
                case .notInstalled:
                    Text(CoreMLBiRefNet.download == nil
                         ? "Not installed. It is included in the full CutOut download; you can also install the CutOut Core ML model file (.zip or .mlpackage) here."
                         : "Not installed.")
                        .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                case .disabled:
                    Text("Installed but turned off. CutOut uses the ONNX model on the CPU.").font(.callout).foregroundStyle(.secondary)
                case .unsupported:
                    Text("Requires Apple Silicon. This Mac uses the ONNX model on the CPU.").font(.callout).foregroundStyle(.secondary)
                case .failed(let why):
                    Text("Could not be used this session: \(why). CutOut falls back to ONNX automatically.")
                        .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
                Text("Licence: MIT (ZhengPeng7/BiRefNet) · weights SHA-256 verified").font(.caption).foregroundStyle(.tertiary)
                if manager.installingCoreML { ProgressView("Verifying and installing…").controlSize(.small) }
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 6) {
                if CoreMLBiRefNet.isSupportedHardware {
                    Toggle("Use", isOn: $useCoreML)
                        .toggleStyle(.switch).controlSize(.small)
                        .onChange(of: useCoreML) { manager.unload(); refresh += 1 }
                    if case .ready(.installed) = coreMLState {
                        Button("Delete", role: .destructive) { manager.deleteCoreML(); refresh += 1 }.controlSize(.small)
                    } else if case .notInstalled = coreMLState {
                        Button("Install from File…") { pickArchive() }
                            .disabled(manager.installingCoreML)
                    }
                }
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(.quaternary.opacity(0.5)))
    }

    private func pickArchive() {
        let p = NSOpenPanel()
        p.allowedContentTypes = [.zip, .package, .folder]
        p.canChooseDirectories = true
        p.treatsFilePackagesAsDirectories = false
        p.message = "Choose the CutOut BiRefNet Core ML model (.zip or .mlpackage)"
        guard p.runModal() == .OK, let url = p.url else { return }
        Task {
            await manager.installCoreML(from: url)
            refresh += 1
        }
    }

    @ViewBuilder private func onnxRow(_ spec: ModelSpec) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: manager.isInstalled(spec) ? "checkmark.circle.fill" : "arrow.down.circle")
                .foregroundStyle(manager.isInstalled(spec) ? Color.green : Color.secondary)
            VStack(alignment: .leading, spacing: 2) {
                HStack {
                    Text(spec.title).fontWeight(.medium)
                    Text(spec.sizeLabel).foregroundStyle(.secondary)
                    if bestModel == spec.id { Text("Preferred").font(.caption).padding(.horizontal, 6).padding(.vertical, 1).background(.tint.opacity(0.15), in: Capsule()) }
                }
                Text(spec.detail).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                if manager.downloadingSpec == spec, let p = manager.downloadProgress {
                    ProgressView(value: p) { Text("Downloading… \(Int(p * 100))%").font(.caption) }
                }
            }
            Spacer()
            if manager.isInstalled(spec) {
                if bestModel != spec.id { Button("Prefer") { bestModel = spec.id; manager.unload() }.controlSize(.small) }
                if ModelManager.bundledURL(spec) == nil {
                    Button("Delete", role: .destructive) { manager.delete(spec) }.controlSize(.small)
                }
            } else if manager.downloadingSpec == spec {
                Button("Cancel") { manager.cancelDownload() }.controlSize(.small)
            } else {
                Button("Download") { manager.download(spec) }
                    .controlSize(.small)
                    .disabled(manager.downloadingSpec != nil)
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 6)
    }
}

struct ResizeSheet: View {
    enum Mode { case image, canvas }
    let mode: Mode
    @Environment(EditorModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var width: Int = 0
    @State private var height: Int = 0
    @State private var keepAspect = true
    @State private var anchor = 4

    var body: some View {
        let w0 = max(1, model.state.width), h0 = max(1, model.state.height)
        VStack(alignment: .leading, spacing: 14) {
            Text(mode == .image ? "Resize Image" : "Canvas Size").font(.title3.weight(.semibold))
            Text("Current: \(w0) × \(h0) px").foregroundStyle(.secondary)
            Grid(alignment: .leading, verticalSpacing: 8) {
                GridRow {
                    Text("Width")
                    TextField("", value: $width, format: .number).frame(width: 100)
                        .onChange(of: width) { if keepAspect && mode == .image { let nh = Int((Double(width) * Double(h0) / Double(w0)).rounded()); if nh != height { height = nh } } }
                    Text("px")
                }
                GridRow {
                    Text("Height")
                    TextField("", value: $height, format: .number).frame(width: 100)
                        .onChange(of: height) { if keepAspect && mode == .image { let nw = Int((Double(height) * Double(w0) / Double(h0)).rounded()); if nw != width { width = nw } } }
                    Text("px")
                }
            }
            if mode == .image {
                Toggle("Keep proportions", isOn: $keepAspect)
                HStack {
                    ForEach([25, 50, 200], id: \.self) { p in
                        Button("\(p)%") { width = w0 * p / 100; height = h0 * p / 100 }
                    }
                }
                .controlSize(.small)
            } else {
                Text("Anchor").font(.callout)
                Grid(horizontalSpacing: 4, verticalSpacing: 4) {
                    ForEach(0..<3) { r in
                        GridRow {
                            ForEach(0..<3) { c in
                                Button { anchor = r * 3 + c } label: {
                                    Image(systemName: anchor == r * 3 + c ? "circle.fill" : "circle").frame(width: 18, height: 18)
                                }
                                .buttonStyle(.borderless)
                            }
                        }
                    }
                }
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(mode == .image ? "Resize" : "Apply") {
                    if mode == .image {
                        model.run("resize_image", ["width": width, "height": height])
                    } else {
                        model.run("canvas_size", ["width": width, "height": height, "anchor_x": Double(anchor % 3) / 2, "anchor_y": Double(anchor / 3) / 2])
                    }
                    model.canvas?.zoomToFit()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(width < 1 || height < 1 || width > 16384 || height > 16384)
            }
        }
        .padding(22)
        .frame(width: 340)
        .onAppear { width = w0; height = h0 }
    }
}

struct TextSheet: View {
    let layerID: Int64?
    let point: CGPoint
    @Environment(EditorModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var align = "left"

    var body: some View {
        @Bindable var model = model
        VStack(alignment: .leading, spacing: 12) {
            Text(layerID == nil ? "Add Text" : "Edit Text").font(.title3.weight(.semibold))
            TextEditor(text: $text)
                .font(.system(size: 15))
                .frame(minHeight: 90)
                .scrollContentBackground(.hidden)
                .padding(6)
                .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(.background))
                .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator))
            HStack {
                FontPicker(name: $model.settings.fontName).frame(maxWidth: 220)
                TextField("Size", value: $model.settings.fontSize, format: .number).frame(width: 56)
                Text("pt")
            }
            HStack {
                Toggle("Bold", isOn: $model.settings.bold)
                Toggle("Italic", isOn: $model.settings.italic)
                Picker("", selection: $align) {
                    Image(systemName: "text.alignleft").tag("left")
                    Image(systemName: "text.aligncenter").tag("center")
                    Image(systemName: "text.alignright").tag("right")
                }
                .pickerStyle(.segmented)
                .frame(width: 120)
                ColorRow(title: "", rgba: $model.settings.color)
            }
            .toggleStyle(.button)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(layerID == nil ? "Add" : "Update") {
                    let s = model.settings
                    let props = TextProps(text: text, font: s.fontName, size: s.fontSize, color: s.colorRGBA.map { UInt8(clamping: $0) },
                                          bold: s.bold, italic: s.italic, align: align)
                    model.commitText(props, layerID: layerID, at: point)
                    dismiss()
                }
                .keyboardShortcut(.return, modifiers: .command)
                .buttonStyle(.borderedProminent)
                .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 460)
        .onAppear {
            if let id = layerID, let l = model.state.layers.first(where: { $0.id == id }), let t = l.text {
                text = t.text
                align = t.align.isEmpty ? "left" : t.align
                model.settings.fontName = t.font
                model.settings.fontSize = t.size
                model.settings.bold = t.bold
                model.settings.italic = t.italic
                model.settings.color = t.color.map { Double($0) / 255 }
            }
        }
    }
}

struct JPEGWarningSheet: View {
    let action: CopyAction
    @Environment(EditorModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var dontAsk = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("JPEG doesn't support transparency", systemImage: "exclamationmark.triangle")
                .font(.headline)
            Text("Transparent areas will be filled with white. Copy as PNG to keep the transparent background.")
                .foregroundStyle(.secondary)
            Toggle("Don't ask again", isOn: $dontAsk)
            HStack {
                Button("Copy as PNG") {
                    dismiss()
                    model.copy(action, format: .png)
                }
                Button("Set PNG as Default") {
                    model.clipboardFormat = .png
                    dismiss()
                    model.copy(action, format: .png)
                }
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Copy JPEG") {
                    if dontAsk { UserDefaults.standard.set(true, forKey: "jpegNoWarn") }
                    dismiss()
                    model.copy(action, format: .jpeg, force: true)
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 480)
    }
}

struct SettingsView: View {
    @AppStorage("clipboardFormat") private var clipboardFormat = "png"
    @AppStorage("jpegNoWarn") private var jpegNoWarn = false
    @AppStorage("appearance") private var appearance = AppearancePreference.system.rawValue
    @State private var manager = ModelManager.shared

    var body: some View {
        TabView {
            Form {
                Picker("Appearance", selection: $appearance) {
                    ForEach(AppearancePreference.allCases) { a in Label(a.title, systemImage: a.symbol).tag(a.rawValue) }
                }
                .pickerStyle(.segmented)
                Picker("Default copy format", selection: $clipboardFormat) {
                    Text("PNG — keeps transparency").tag("png")
                    Text("JPEG — white background").tag("jpeg")
                }
                Toggle("Warn when JPEG would discard transparency", isOn: Binding(get: { !jpegNoWarn }, set: { jpegNoWarn = !$0 }))
                Text("Copies always include a TIFF representation with alpha for apps such as Keynote and PowerPoint.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .formStyle(.grouped)
            .tabItem { Label("General", systemImage: "gearshape") }

            Form {
                Toggle("Use Core ML when available (Apple Silicon, GPU)", isOn: Binding(
                    get: { UserDefaults.standard.object(forKey: "useCoreML") as? Bool ?? true },
                    set: { UserDefaults.standard.set($0, forKey: "useCoreML"); manager.unload() }))
                LabeledContent("Core ML model") {
                    Text(CoreMLBiRefNet.locate()?.label ?? "Not installed").foregroundStyle(.secondary)
                }
                LabeledContent("Fallback") { Text("ONNX Runtime · all CPU cores").foregroundStyle(.secondary) }
                ForEach(ModelSpec.all) { spec in
                    LabeledContent(spec.title) {
                        Text(manager.isInstalled(spec) ? "Installed" : "Not installed (\(spec.sizeLabel))").foregroundStyle(.secondary)
                    }
                }
                LabeledContent("Fast mode") { Text("Apple Vision (built in)").foregroundStyle(.secondary) }
                Button("Show Models Folder") {
                    try? FileManager.default.createDirectory(at: ModelManager.modelsDirectory, withIntermediateDirectories: true)
                    NSWorkspace.shared.open(ModelManager.modelsDirectory)
                }
                Text("CutOut never sends images anywhere. The only network access is a model download you start yourself.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .formStyle(.grouped)
            .tabItem { Label("AI Models", systemImage: "cpu") }
        }
        .frame(width: 480, height: 320)
    }
}
