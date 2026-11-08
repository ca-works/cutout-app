import SwiftUI

struct ContentView: View {
    @Environment(EditorModel.self) private var model
    @AppStorage("showTools") private var showTools = false
    @AppStorage("showLayers") private var showLayers = false
    @AppStorage("appearance") private var appearance = AppearancePreference.system.rawValue
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        @Bindable var model = model
        // The tool panel is a plain in-window panel rather than a NavigationSplitView sidebar column:
        // on current macOS the column's hosting view draws its content at the toolbar-inset position but
        // hit-tests ~toolbar-height higher, so clicks missed the icons they were aimed at.
        HStack(spacing: 0) {
            if showTools && !model.isEmpty {
                ToolSidebar()
                    .frame(width: 260)
                    .background { SidebarMaterial().ignoresSafeArea() }
                    .overlay(alignment: .trailing) { Divider().ignoresSafeArea() }
                    .transition(reduceMotion ? .opacity : .move(edge: .leading))
            }
            CanvasArea()
                .inspector(isPresented: Binding(get: { showLayers && !model.isEmpty }, set: { showLayers = $0 })) {
                    LayersPanel()
                        .inspectorColumnWidth(min: 230, ideal: 260, max: 340)
                }
        }
        .motion(.smooth(duration: 0.28), value: showTools)
        // Opening or closing a panel changes the canvas rectangle: re-fit and re-centre the document in what is
        // left. `fitted` stays on afterwards, so the canvas keeps tracking the panel's animation frame by frame
        // (no jump, no accumulated drift). Manual zoom/pan afterwards behaves as before.
        .onChange(of: showTools) { _, _ in model.canvas?.fitToViewport() }
        .onChange(of: showLayers) { _, _ in model.canvas?.fitToViewport() }
        .toolbar { MainToolbar(showTools: $showTools, showLayers: $showLayers) }
        .navigationTitle(model.isEmpty ? "CutOut" : model.suggestedName)
        .navigationSubtitle(model.isEmpty ? "" : "\(model.state.width) × \(model.state.height)")
        .sheet(item: $model.sheet) { sheet in
            switch sheet {
            case .modelSetup: ModelSetupSheet()
            case .resize: ResizeSheet(mode: .image)
            case .canvasSize: ResizeSheet(mode: .canvas)
            case .text(let id, let p): TextSheet(layerID: id, point: p)
            case .jpegWarning(let action): JPEGWarningSheet(action: action)
            }
        }
        .preferredColorScheme((AppearancePreference(rawValue: appearance) ?? .system).scheme)
        .frame(minWidth: 700, minHeight: 460)
    }
}

/// Canvas plus the floating overlays (empty state, progress, toasts).
struct CanvasArea: View {
    @Environment(EditorModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            Theme.canvas.ignoresSafeArea()
            CanvasView(model: model)
                .opacity(model.isEmpty ? 0 : 1)
            if model.isEmpty {
                EmptyStateView()
                    .transition(reduceMotion ? .opacity : .opacity.combined(with: .scale(scale: 0.97)))
            } else if model.dropTargeted {
                DropOverlay()
                    .transition(.opacity)
            }
            VStack {
                if let t = model.toast {
                    ToastView(toast: t)
                        .transition(reduceMotion ? .opacity : .move(edge: .top).combined(with: .opacity))
                        .padding(.top, 14)
                }
                Spacer()
                if model.compareOn {
                    Label("Drag across the image to compare before and after", systemImage: "rectangle.split.2x1")
                        .font(.callout)
                        .padding(.horizontal, 14).padding(.vertical, 8)
                        .background(.regularMaterial, in: Capsule())
                        .overlay(Capsule().strokeBorder(Theme.cardStroke, lineWidth: 1))
                        .padding(.bottom, 44)
                        .transition(.opacity)
                }
            }
            .allowsHitTesting(false)
            if let busy = model.busy {
                ProcessingView(busy: busy)
                    .transition(reduceMotion ? .opacity : .opacity.combined(with: .scale(scale: 0.96)))
            }
            if !model.isEmpty {
                VStack {
                    Spacer()
                    StatusBar()
                }
            }
        }
        .motion(.smooth(duration: 0.3), value: model.isEmpty)
        .motion(.snappy(duration: 0.2), value: model.dropTargeted)
        .motion(.smooth(duration: 0.28), value: model.busy)
        .motion(.snappy(duration: 0.2), value: model.compareOn)
    }
}

/// Shown over an open document while a file is dragged in.
struct DropOverlay: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 16, style: .continuous)
            .strokeBorder(Color.accentColor, lineWidth: 2.5)
            .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Color.accentColor.opacity(0.08)))
            .overlay {
                Label("Drop to add as a new layer", systemImage: "square.on.square")
                    .font(.headline)
                    .padding(.horizontal, 18).padding(.vertical, 10)
                    .background(.regularMaterial, in: Capsule())
            }
            .padding(12)
            .allowsHitTesting(false)
    }
}

struct EmptyStateView: View {
    @Environment(EditorModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var hover = false
    @State private var appeared = false

    private var active: Bool { hover || model.dropTargeted }

    var body: some View {
        // The wordmark on top, as on the website, sized to the canvas: as wide as ~78 % of it, but
        // never taller than about a fifth of the height, so the card below always fits.
        GeometryReader { geo in
            let size = max(44, min(geo.size.width * 0.78 / 3.65, geo.size.height * 0.19, 168))
            VStack(spacing: max(14, size * 0.2)) {
                Wordmark(size: size)
                    .opacity(appeared || reduceMotion ? 1 : 0)
                    .offset(y: appeared || reduceMotion ? 0 : 10)
                card
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onAppear { withAnimation(reduceMotion ? nil : .smooth(duration: 0.5)) { appeared = true } }
    }

    private var card: some View {
        VStack(spacing: 22) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .interpolation(.high)
                .frame(width: 84, height: 84)
                .shadow(color: Color(red: 0.35, green: 0.25, blue: 0.9).opacity(active ? 0.45 : 0.22), radius: active ? 22 : 14, y: 8)
            .scaleEffect(active ? 1.05 : 1)

            VStack(spacing: 8) {
                Text(model.dropTargeted ? "Drop to open" : "Remove any background")
                    .font(.system(size: 26, weight: .semibold, design: .rounded))
                    .contentTransition(.interpolate)
                HStack(spacing: 5) {
                    Text("Paste with")
                    KeyCap("⌘")
                    KeyCap("V")
                    Text("or drop an image here")
                }
                .font(.callout)
                .foregroundStyle(.secondary)
            }

            HStack(spacing: 10) {
                Button { model.paste() } label: {
                    Label("Paste Image", systemImage: "doc.on.clipboard").frame(minWidth: 108)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                Button { model.openPanel() } label: {
                    Label("Open…", systemImage: "folder").frame(minWidth: 70)
                }
                .controlSize(.large)
            }

            VStack(spacing: 10) {
                if model.hasRecovery {
                    Button("Restore Last Session") { model.restoreLastSession() }
                        .buttonStyle(.link)
                        .font(.callout)
                }
                Label("Runs entirely on this Mac. Nothing is uploaded.", systemImage: "lock.fill")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, 54).padding(.vertical, 44)
        .frame(maxWidth: 520)
        .background {
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .fill(Theme.card)
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .strokeBorder(active ? Color.accentColor.opacity(0.8) : Theme.cardStroke,
                              style: StrokeStyle(lineWidth: active ? 2 : 1.5, dash: active ? [] : [7, 6]))
        }
        .shadow(color: .black.opacity(0.06), radius: 24, y: 10)
        .scaleEffect(appeared || reduceMotion ? 1 : 0.985)
        .opacity(appeared || reduceMotion ? 1 : 0)
        .onHover { hover = $0 }
        .motion(.smooth(duration: 0.25), value: active)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("No image. Paste or drop an image, or open a file.")
    }
}

struct KeyCap: View {
    let key: String
    init(_ k: String) { key = k }
    var body: some View {
        Text(key)
            .font(.system(.callout, design: .rounded).weight(.medium))
            .padding(.horizontal, 6).padding(.vertical, 1)
            .background(RoundedRectangle(cornerRadius: 5, style: .continuous).fill(Theme.hover))
            .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous).strokeBorder(Theme.cardStroke, lineWidth: 0.5))
    }
}

struct ToastView: View {
    let toast: Toast
    var body: some View {
        Label(toast.message, systemImage: toast.symbol)
            .font(.callout.weight(.medium))
            .symbolRenderingMode(.multicolor)
            .padding(.horizontal, 16).padding(.vertical, 9)
            .background(.regularMaterial, in: Capsule())
            .overlay(Capsule().strokeBorder(toast.isError ? Color.red.opacity(0.45) : Theme.cardStroke, lineWidth: 1))
            .shadow(color: .black.opacity(0.14), radius: 10, y: 3)
            .accessibilityAddTraits(.updatesFrequently)
    }
}

struct ProcessingView: View {
    @Environment(EditorModel.self) private var model
    let busy: BusyState

    var body: some View {
        HStack(spacing: 14) {
            ProgressView().controlSize(.small)
            VStack(alignment: .leading, spacing: 2) {
                Text(busy.title).font(.callout.weight(.medium))
                Text("Processing on this Mac").font(.caption).foregroundStyle(.secondary)
            }
            if busy.cancellable {
                Button("Cancel") { model.cancelRemoval() }
                    .keyboardShortcut(.cancelAction)
                    .controlSize(.small)
            }
        }
        .padding(.horizontal, 18).padding(.vertical, 13)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Theme.cardStroke, lineWidth: 1))
        .shadow(color: .black.opacity(0.18), radius: 18, y: 6)
        .accessibilityElement(children: .combine)
    }
}

struct StatusBar: View {
    @Environment(EditorModel.self) private var model

    var body: some View {
        HStack(spacing: 12) {
            Text(model.tool.help)
                .lineLimit(1)
                .foregroundStyle(.secondary)
            Spacer(minLength: 8)
            BackendPill()
            if let l = model.state.activeLayer {
                Text(l.name).lineLimit(1).foregroundStyle(.secondary)
            }
            Divider().frame(height: 12)
            Menu {
                Button("Fit in Window") { model.canvas?.zoomToFit() }
                Button("Actual Size (100%)") { model.canvas?.setZoom(1) }
                Divider()
                ForEach([0.25, 0.5, 2, 4, 8], id: \.self) { z in
                    Button("\(Int(z * 100))%") { model.canvas?.setZoom(z) }
                }
            } label: {
                Text("\(Int((model.zoom * 100).rounded()))%")
                    .monospacedDigit()
                    .frame(minWidth: 44, alignment: .trailing)
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .help("Zoom (⌘+ / ⌘− / ⌘0 fit / ⌘1 actual size)")
        }
        .font(.caption)
        .padding(.horizontal, 14)
        .frame(height: 28)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
    }
}

/// Which engine will remove the background: Core ML (bundled/installed),
/// ONNX fallback, or the built-in Fast mode only.
struct BackendPill: View {
    private let manager = ModelManager.shared

    var body: some View {
        let (text, symbol): (String, String) = {
            if let b = manager.activeBackend { return (b, "cpu") }
            if CoreMLBiRefNet.locate() != nil { return ("Core ML ready", "bolt.fill") }
            if manager.hasBestModel { return ("ONNX (CPU)", "cpu") }
            return ("Fast mode", "hare")
        }()
        Label(text, systemImage: symbol)
            .labelStyle(.titleAndIcon)
            .font(.caption2.weight(.medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 8).padding(.vertical, 2)
            .background(Capsule().fill(Theme.hover))
            .help("Background-removal engine. Everything runs offline on this Mac.")
    }
}

/// The standard translucent sidebar material.
struct SidebarMaterial: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let v = NSVisualEffectView()
        v.material = .sidebar
        v.blendingMode = .behindWindow
        v.state = .followsWindowActiveState
        return v
    }
    func updateNSView(_ v: NSVisualEffectView, context: Context) {}
}
