import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct Toast: Identifiable, Equatable {
    let id = UUID()
    var message: String
    var symbol: String
    var isError: Bool = false
}

struct BusyState: Equatable {
    var title: String
    var detail: String?
    var cancellable: Bool
}

enum ActiveSheet: Identifiable {
    case modelSetup
    case resize
    case canvasSize
    case text(layerID: Int64?, point: CGPoint)
    case jpegWarning(action: CopyAction)

    var id: String {
        switch self {
        case .modelSetup: "model"
        case .resize: "resize"
        case .canvasSize: "canvas"
        case .text: "text"
        case .jpegWarning: "jpeg"
        }
    }
}

enum CopyAction: Equatable {
    case copy, copyAndClear, copyAndClose
}

enum Panel: String {
    case none, refine, adjust
}

@MainActor
@Observable
final class EditorModel {
    let engine = EngineBridge()
    private(set) var state = EngineState()

    var tool: Tool = Tool(rawValue: UserDefaults.standard.string(forKey: "tool") ?? "") ?? .move {
        didSet {
            UserDefaults.standard.set(tool.rawValue, forKey: "tool")
            canvas?.toolChanged()
        }
    }
    var settings: ToolSettings = EditorModel.loadSettings() {
        didSet { saveSettings() }
    }
    var selectionMode: SelectionMode = .replace
    var toast: Toast?
    var busy: BusyState?
    var zoom: Double = 1
    var compareOn = false {
        didSet { canvas?.setCompare(compareOn) }
    }
    var dropTargeted = false
    var revealToken = 0
    var sheet: ActiveSheet?
    var panel: Panel = .none
    var selectedLayerIDs: Set<Int64> = []
    var projectURL: URL?
    var outlineVersion = 0
    private(set) var outline: [Float] = []
    private var thumbs: [Int64: (key: Int, image: NSImage)] = [:]
    weak var canvas: CanvasNSView?
    private var removalTask: Task<Void, Never>?
    private var activeModel: (any Segmenter)?
    private var toastTask: Task<Void, Never>?
    private var lastSelectionState: (Bool, CGRect?, Int) = (false, nil, -1)

    var isEmpty: Bool { state.isEmpty }
    var canvasSize: CGSize { CGSize(width: state.width, height: state.height) }

    init() {}

    // MARK: Settings persistence

    private static func loadSettings() -> ToolSettings {
        guard let d = UserDefaults.standard.data(forKey: "toolSettings"),
              let s = try? JSONDecoder().decode(ToolSettings.self, from: d) else { return ToolSettings() }
        return s
    }

    private func saveSettings() {
        if let d = try? JSONEncoder().encode(settings) { UserDefaults.standard.set(d, forKey: "toolSettings") }
    }

    var clipboardFormat: ImageCodec.Format {
        get { ImageCodec.Format(rawValue: UserDefaults.standard.string(forKey: "clipboardFormat") ?? "png") ?? .png }
        set { UserDefaults.standard.set(newValue.rawValue, forKey: "clipboardFormat") }
    }

    // MARK: Engine plumbing

    /// Push pending pixels to the canvas and refresh UI state.
    func refresh(full: Bool = false) {
        canvas?.updateFromEngine(full: full)
        let newState = engine.state()
        let selKey = (newState.hasSelection, newState.selectionBounds, newState.version)
        if newState.hasSelection != lastSelectionState.0 || newState.selectionBounds != lastSelectionState.1 || newState.version != lastSelectionState.2 || full {
            outline = newState.hasSelection ? engine.selectionOutline() : []
            outlineVersion += 1
            canvas?.updateSelectionOutline()
        }
        lastSelectionState = selKey
        if newState != state { state = newState }
        selectedLayerIDs = selectedLayerIDs.filter { id in state.layers.contains { $0.id == id } }
        canvas?.updateOverlays()
    }

    /// Run an engine command and refresh. Errors appear as a toast.
    @discardableResult
    func run(_ cmd: String, _ args: [String: Any] = [:], refresh doRefresh: Bool = true, silent: Bool = false) -> [String: Any]? {
        switch engine.command(cmd, args) {
        case .success(let r):
            if doRefresh { refresh() }
            return r
        case .failure(let e):
            if !silent { showToast(e.message, symbol: "exclamationmark.triangle.fill", error: true) }
            if doRefresh { refresh() }
            return nil
        }
    }

    func showToast(_ message: String, symbol: String = "checkmark.circle.fill", error: Bool = false) {
        toastTask?.cancel()
        withAnimation(.snappy(duration: 0.25)) { toast = Toast(message: message, symbol: symbol, isError: error) }
        toastTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(error ? 3.5 : 1.8))
            guard !Task.isCancelled else { return }
            withAnimation(.easeOut(duration: 0.3)) { self?.toast = nil }
        }
    }

    func thumbnail(for layer: LayerInfo) -> NSImage? {
        let key = state.version &+ layer.id.hashValue
        if let t = thumbs[layer.id], t.key == key { return t.image }
        guard let img = engine.thumbnail(layer.id, max: 96), let cg = ImageCodec.cgImage(ImageCodec.premultiply(img), premultiplied: true) else { return nil }
        let ns = NSImage(cgImage: cg, size: NSSize(width: img.width, height: img.height))
        thumbs[layer.id] = (key, ns)
        return ns
    }

    // MARK: Import

    func openPanel() {
        let p = NSOpenPanel()
        p.allowedContentTypes = ImageCodec.readableTypes + [UTType.cutoutProject]
        p.allowsMultipleSelection = true
        p.message = "Choose images or a CutOut project"
        guard p.runModal() == .OK else { return }
        open(urls: p.urls, asLayers: !isEmpty)
    }

    func open(urls: [URL], asLayers: Bool) {
        var first = !asLayers
        for url in urls {
            if url.pathExtension.lowercased() == "cutout" {
                openProject(url)
                first = false
                continue
            }
            do {
                let r = try ImageCodec.decode(url: url)
                try importImage(r.image, name: url.deletingPathExtension().lastPathComponent, asLayer: !first)
                if r.downscaled { showToast("Very large image was downscaled to fit 120 MP", symbol: "arrow.down.right.and.arrow.up.left") }
                first = false
            } catch {
                showToast(error.localizedDescription, symbol: "exclamationmark.triangle.fill", error: true)
            }
        }
    }

    func importImage(_ img: RGBAImage, name: String, asLayer: Bool) throws {
        let wasEmpty = isEmpty
        _ = try engine.importImage(img, name: name, asLayer: asLayer && !wasEmpty)
        if wasEmpty || !asLayer { projectURL = nil }
        refresh(full: wasEmpty || !asLayer)
        if wasEmpty || !asLayer { canvas?.zoomToFit(animated: false) }
    }

    /// ⌘V: paste as a new layer (or new document when empty).
    func paste(asNewDocument: Bool = false) {
        do {
            let items = try Clipboard.read()
            for (i, item) in items.enumerated() {
                try importImage(item.image, name: item.name, asLayer: !(asNewDocument && i == 0))
            }
            if !isEmpty && !asNewDocument && state.layers.count > 1 {
                showToast("Pasted as a new layer", symbol: "square.on.square")
            }
        } catch {
            showToast(error.localizedDescription, symbol: "doc.on.clipboard", error: true)
        }
    }

    // MARK: Copy actions

    func copy(_ action: CopyAction, format: ImageCodec.Format? = nil, force: Bool = false) {
        guard !isEmpty, let img = engine.export() else {
            showToast("Nothing to copy yet", symbol: "doc.on.clipboard", error: true)
            return
        }
        let fmt = format ?? clipboardFormat
        if fmt == .jpeg && !force && img.hasTransparency && !UserDefaults.standard.bool(forKey: "jpegNoWarn") {
            sheet = .jpegWarning(action: action)
            return
        }
        do {
            try Clipboard.write(img, format: fmt)
        } catch {
            // Never clear or close if the clipboard write failed.
            showToast("Copy failed: \(error.localizedDescription)", symbol: "exclamationmark.triangle.fill", error: true)
            return
        }
        switch action {
        case .copy:
            showToast("Copied \(fmt.label) to clipboard", symbol: "checkmark.circle.fill")
        case .copyAndClear:
            clearWorkspace(saveRecovery: true)
            showToast("Copied — workspace cleared", symbol: "checkmark.circle.fill")
        case .copyAndClose:
            saveRecovery()
            NSApp.terminate(nil)
        }
    }

    // MARK: Workspace

    func clearWorkspace(saveRecovery doSave: Bool = true) {
        if doSave { saveRecovery() }
        cancelRemoval()
        engine.command("clear")
        projectURL = nil
        compareOn = false
        panel = .none
        thumbs = [:]
        selectedLayerIDs = []
        refresh(full: true)
    }

    static var recoveryURL: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("CutOut/Recovery/Last Session.cutout")
    }

    var hasRecovery: Bool { FileManager.default.fileExists(atPath: Self.recoveryURL.path) }

    /// Quietly keep the last workspace so Copy & Clear / Copy & Close and
    /// quitting never lose work without asking.
    func saveRecovery() {
        guard !isEmpty, state.modified || projectURL == nil else { return }
        let url = Self.recoveryURL
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        _ = engine.command("save_project", ["path": url.path, "recovery": true])
    }

    func restoreLastSession() {
        guard hasRecovery else { return }
        openProject(Self.recoveryURL, isRecovery: true)
    }

    // MARK: Projects and export

    func openProject(_ url: URL, isRecovery: Bool = false) {
        switch engine.command("open_project", ["path": url.path]) {
        case .success:
            projectURL = isRecovery ? nil : url
            thumbs = [:]
            refresh(full: true)
            canvas?.zoomToFit(animated: false)
            if !isRecovery { NSDocumentController.shared.noteNewRecentDocumentURL(url) }
        case .failure(let e):
            showToast(e.message, symbol: "exclamationmark.triangle.fill", error: true)
        }
    }

    func openProjectPanel() {
        let p = NSOpenPanel()
        p.allowedContentTypes = [UTType.cutoutProject]
        guard p.runModal() == .OK, let url = p.url else { return }
        openProject(url)
    }

    func saveProject(as: Bool = false) {
        guard !isEmpty else { return }
        var url = projectURL
        if url == nil || `as` {
            let p = NSSavePanel()
            p.allowedContentTypes = [UTType.cutoutProject]
            p.nameFieldStringValue = suggestedName + ".cutout"
            guard p.runModal() == .OK, let u = p.url else { return }
            url = u
        }
        guard let url else { return }
        switch engine.command("save_project", ["path": url.path]) {
        case .success:
            projectURL = url
            NSDocumentController.shared.noteNewRecentDocumentURL(url)
            refresh()
            showToast("Saved \(url.lastPathComponent)", symbol: "checkmark.circle.fill")
        case .failure(let e):
            showToast("Save failed: \(e.message)", symbol: "exclamationmark.triangle.fill", error: true)
        }
    }

    var suggestedName: String {
        if let p = projectURL { return p.deletingPathExtension().lastPathComponent }
        let n = state.layers.first?.name ?? "Image"
        return n.isEmpty ? "CutOut Image" : n
    }

    func export(format: ImageCodec.Format, scale: Double = 1) {
        guard !isEmpty, var img = engine.export() else { return }
        if scale != 1 {
            let w = max(1, Int((Double(img.width) * scale).rounded())), h = max(1, Int((Double(img.height) * scale).rounded()))
            guard let r = ImageCodec.resized(img, width: w, height: h) else { return }
            img = r
        }
        let p = NSSavePanel()
        p.allowedContentTypes = [format.utType]
        p.nameFieldStringValue = suggestedName + (format == .png ? " (cutout)" : "") + "." + format.ext
        p.canCreateDirectories = true
        guard p.runModal() == .OK, let url = p.url else { return }
        guard let data = ImageCodec.encode(img, as: format) else {
            showToast("Could not encode the image", symbol: "exclamationmark.triangle.fill", error: true)
            return
        }
        do {
            try data.write(to: url, options: .atomic)
            showToast("Exported \(url.lastPathComponent)", symbol: "square.and.arrow.up")
        } catch {
            showToast("Export failed: \(error.localizedDescription)", symbol: "exclamationmark.triangle.fill", error: true)
        }
    }

    // MARK: Undo

    func undo() {
        if case .success(let r) = engine.command("undo") {
            refresh()
            if let l = r["label"] as? String { canvas?.announce("Undo \(l)") }
        }
    }

    func redo() {
        if case .success = engine.command("redo") { refresh() }
    }

    // MARK: Background removal

    var isRemoving: Bool { removalTask != nil }

    func removeBackground(_ mode: RemovalMode = .best) {
        guard !isEmpty, let layer = state.activeLayer ?? state.layers.last else { return }
        guard removalTask == nil else { return }
        let manager = ModelManager.shared
        if mode == .best && !manager.hasBestModel {
            sheet = .modelSetup
            return
        }
        guard let pixels = engine.layerPixels(layer.id) else { return }
        let layerID = layer.id
        busy = BusyState(title: mode == .best ? "Removing background…" : "Removing background (Fast)…", detail: nil, cancellable: true)
        removalTask = Task { [weak self] in
            let started = Date()
            do {
                let alpha: [UInt8]
                if mode == .best {
                    let model = try await manager.bestSegmenter()
                    await MainActor.run { self?.activeModel = model }
                    alpha = try await Task.detached(priority: .userInitiated) {
                        try model.predict(pixels, refine: true)
                    }.value
                    manager.activeBackend = model.backend
                } else {
                    alpha = try await Task.detached(priority: .userInitiated) {
                        let a = try VisionRemover.alpha(for: pixels)
                        return refineAlpha(pixels, alpha: a, radius: 3)
                    }.value
                    manager.activeBackend = "Apple Vision"
                }
                if mode == .best {
                    manager.scheduleUnload()
                    malloc_zone_pressure_relief(nil, 0) // hand inference scratch memory back to the OS
                }
                try Task.checkCancellation()
                guard let self else { return }
                try self.engine.setLayerMask(layerID, alpha: alpha, width: pixels.width, height: pixels.height, label: "Remove Background")
                self.busy = nil
                self.removalTask = nil
                self.activeModel = nil
                self.revealToken += 1
                self.canvas?.prepareReveal()
                self.refresh()
                let secs = Date().timeIntervalSince(started)
                self.showToast(String(format: "Background removed in %.1f s", secs), symbol: "sparkles")
            } catch {
                guard let self else { return }
                self.busy = nil
                self.removalTask = nil
                self.activeModel = nil
                let msg = error.localizedDescription
                if !(error is CancellationError) && msg != "Cancelled" {
                    self.showToast(msg, symbol: "exclamationmark.triangle.fill", error: true)
                }
            }
        }
    }

    func cancelRemoval() {
        activeModel?.cancel()
        removalTask?.cancel()
        removalTask = nil
        busy = nil
    }

    // MARK: Layers

    func selectLayer(_ id: Int64, extend: Bool = false) {
        if extend {
            if selectedLayerIDs.contains(id) { selectedLayerIDs.remove(id) } else { selectedLayerIDs.insert(id) }
        } else {
            selectedLayerIDs = [id]
        }
        run("select_layer", ["id": id])
    }

    func mergeSelectedOrDown() {
        if selectedLayerIDs.count >= 2 {
            run("merge_layers", ["ids": Array(selectedLayerIDs)])
            selectedLayerIDs = []
        } else if let a = state.active {
            run("merge_down", ["id": a])
        }
    }

    func setOpacity(_ id: Int64, _ value: Double, begin: Bool) {
        if begin { run("checkpoint", ["label": "Layer Opacity"], refresh: false) }
        run("set_layer", ["id": id, "opacity": value, "record": false])
    }

    // MARK: Text

    func commitText(_ props: TextProps, layerID: Int64?, at point: CGPoint) {
        guard let img = TextRenderer.render(props) else { return }
        do {
            let id = try engine.textLayer(updateID: layerID ?? 0, props: props, image: img, at: point)
            selectedLayerIDs = [id]
            refresh()
        } catch {
            showToast(error.localizedDescription, symbol: "exclamationmark.triangle.fill", error: true)
        }
    }
}

extension UTType {
    static let cutoutProject = UTType(exportedAs: "com.ca-works.cutout.project", conformingTo: .data)
}
