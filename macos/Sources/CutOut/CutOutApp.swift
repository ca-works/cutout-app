import AppKit
import SwiftUI

@main
struct CutOutApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var model = EditorModel()

    var body: some Scene {
        Window("CutOut", id: "main") {
            ContentView()
                .environment(model)
                .onAppear {
                    delegate.model = model
                    StartupProbe.windowAppeared()
                    SelfTest.runIfRequested(model: model)
                    HitTest.runIfRequested(model: model)
                    HitTest.startClickLogIfRequested(model: model)
                }
        }
        .defaultSize(width: 1180, height: 780)
        .windowToolbarStyle(.unified)
        .commands { AppCommands(model: model) }

        Settings {
            SettingsView()
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var model: EditorModel?
    private var pendingOpen: [URL] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        DispatchQueue.main.async { [weak self] in
            guard let self, let model = self.model, !self.pendingOpen.isEmpty else { return }
            Task { @MainActor in model.open(urls: self.pendingOpen, asLayers: false) }
            self.pendingOpen = []
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    func application(_ application: NSApplication, open urls: [URL]) {
        guard let model else { pendingOpen += urls; return }
        Task { @MainActor in model.open(urls: urls, asLayers: false) }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        // Quitting never blocks on a dialog: unsaved work is kept as a
        // recoverable "Last Session" (File ▸ Restore Last Session).
        MainActor.assumeIsolated { model?.saveRecovery() }
        return .terminateNow
    }
}

struct AppCommands: Commands {
    let model: EditorModel
    @AppStorage("showTools") private var showTools = false
    @AppStorage("showLayers") private var showLayers = false
    @AppStorage("appearance") private var appearance = AppearancePreference.system.rawValue

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Open…") { model.openPanel() }.keyboardShortcut("o")
            Button("Open Project…") { model.openProjectPanel() }.keyboardShortcut("o", modifiers: [.command, .shift])
            Button("Restore Last Session") { model.restoreLastSession() }.disabled(!model.hasRecovery)
            Divider()
            Button("Clear Workspace") { model.clearWorkspace() }.keyboardShortcut(.delete, modifiers: [.command, .shift]).disabled(model.isEmpty)
        }
        CommandGroup(replacing: .saveItem) {
            Button("Save Project") { model.saveProject() }.keyboardShortcut("s").disabled(model.isEmpty)
            Button("Save Project As…") { model.saveProject(as: true) }.keyboardShortcut("s", modifiers: [.command, .shift]).disabled(model.isEmpty)
            Divider()
            Button("Export PNG…") { model.export(format: .png) }.keyboardShortcut("e", modifiers: [.command, .shift]).disabled(model.isEmpty)
            Button("Export JPEG…") { model.export(format: .jpeg) }.disabled(model.isEmpty)
            Menu("Export at Size") {
                ForEach([0.25, 0.5, 2.0], id: \.self) { s in
                    Button("PNG at \(Int(s * 100))%") { model.export(format: .png, scale: s) }
                }
            }
            .disabled(model.isEmpty)
        }
        CommandGroup(replacing: .undoRedo) {
            Button(model.state.undoLabel.map { "Undo \($0)" } ?? "Undo") { model.undo() }.keyboardShortcut("z").disabled(!model.state.canUndo)
            Button(model.state.redoLabel.map { "Redo \($0)" } ?? "Redo") { model.redo() }.keyboardShortcut("z", modifiers: [.command, .shift]).disabled(!model.state.canRedo)
        }
        CommandGroup(replacing: .pasteboard) {
            Button("Copy") { model.copy(.copy) }.keyboardShortcut("c").disabled(model.isEmpty)
            Button("Copy & Clear") { model.copy(.copyAndClear) }.keyboardShortcut("c", modifiers: [.command, .shift]).disabled(model.isEmpty)
            Button("Copy & Close") { model.copy(.copyAndClose) }.keyboardShortcut("c", modifiers: [.command, .option]).disabled(model.isEmpty)
            Button("Copy as JPEG") { model.copy(.copy, format: .jpeg) }.disabled(model.isEmpty)
            Divider()
            Button(model.isEmpty ? "Paste" : "Paste as New Layer") { model.paste() }.keyboardShortcut("v")
            Button("Paste as New Image") { model.paste(asNewDocument: true) }.keyboardShortcut("v", modifiers: [.command, .option])
            Divider()
            Button("Select All") { model.run("select_all") }.keyboardShortcut("a").disabled(model.isEmpty)
            Button("Deselect") { model.run("deselect") }.keyboardShortcut("d").disabled(!model.state.hasSelection)
            Button("Invert Selection") { model.run("invert_selection") }.keyboardShortcut("i", modifiers: [.command, .shift]).disabled(model.isEmpty)
            Button("Delete Selected Pixels") { model.run("delete_selection") }.disabled(!model.state.hasSelection)
        }
        CommandMenu("Image") {
            Button("Remove Background") { model.removeBackground(.best) }.keyboardShortcut("b").disabled(model.isEmpty || model.isRemoving)
            Button("Remove Background (Fast)") { model.removeBackground(.fast) }.keyboardShortcut("b", modifiers: [.command, .shift]).disabled(model.isEmpty || model.isRemoving)
            Button("Refine Edges…") { showTools = true; model.panel = .refine }.disabled(model.isEmpty)
            Button("Compare Before / After") { model.compareOn.toggle() }.keyboardShortcut("\\", modifiers: []).disabled(model.isEmpty)
            Divider()
            Button("Crop to Selection") { model.run("crop_to_selection"); model.canvas?.zoomToFit() }.disabled(!model.state.hasSelection)
            Button("Trim Transparent Pixels") { model.run("trim"); model.canvas?.zoomToFit() }.disabled(model.isEmpty)
            Button("Resize Image…") { model.sheet = .resize }.keyboardShortcut("i", modifiers: [.command, .option]).disabled(model.isEmpty)
            Button("Canvas Size…") { model.sheet = .canvasSize }.keyboardShortcut("c", modifiers: [.command, .option, .shift]).disabled(model.isEmpty)
            Divider()
            Button("Rotate 90° Right") { model.run("rotate_canvas", ["direction": "cw"]); model.canvas?.zoomToFit() }.keyboardShortcut("r", modifiers: [.command]).disabled(model.isEmpty)
            Button("Rotate 90° Left") { model.run("rotate_canvas", ["direction": "ccw"]); model.canvas?.zoomToFit() }.keyboardShortcut("l", modifiers: [.command]).disabled(model.isEmpty)
            Button("Flip Horizontal") { model.run("flip_canvas", ["axis": "h"]) }.disabled(model.isEmpty)
            Button("Flip Vertical") { model.run("flip_canvas", ["axis": "v"]) }.disabled(model.isEmpty)
            Divider()
            Button("Manage AI Models…") { model.sheet = .modelSetup }
        }
        CommandMenu("Layer") {
            Button("New Layer from Clipboard") { model.paste() }.keyboardShortcut("v", modifiers: [.command, .shift])
            Button("Duplicate Layer") { model.run("duplicate_layer") }.keyboardShortcut("j").disabled(model.isEmpty)
            Button("Delete Layer") { model.run("delete_layer") }.disabled(model.state.layers.count <= 1)
            Divider()
            Button("Merge Down / Merge Selected") { model.mergeSelectedOrDown() }.keyboardShortcut("e").disabled(model.state.layers.count <= 1)
            Button("Flatten Image") { model.run("flatten") }.disabled(model.isEmpty)
            Divider()
            Button("Bring to Front") { if let a = model.state.active { model.run("move_layer", ["id": a, "direction": "top"]) } }.keyboardShortcut("]", modifiers: [.command, .shift]).disabled(model.isEmpty)
            Button("Bring Forward") { if let a = model.state.active { model.run("move_layer", ["id": a, "direction": "up"]) } }.keyboardShortcut("]").disabled(model.isEmpty)
            Button("Send Backward") { if let a = model.state.active { model.run("move_layer", ["id": a, "direction": "down"]) } }.keyboardShortcut("[").disabled(model.isEmpty)
            Button("Send to Back") { if let a = model.state.active { model.run("move_layer", ["id": a, "direction": "bottom"]) } }.keyboardShortcut("[", modifiers: [.command, .shift]).disabled(model.isEmpty)
            Divider()
            Button("Restore All Erased Pixels") { model.run("clear_mask") }.disabled(model.isEmpty)
            Button("Apply Mask") { model.run("apply_mask") }.disabled(model.isEmpty)
            Button("Rasterize Text") { model.run("rasterize_layer") }.disabled(!(model.state.activeLayer?.isText ?? false))
        }
        CommandGroup(after: .toolbar) {
            Button("Zoom In") { model.canvas?.zoomIn() }.keyboardShortcut("+").disabled(model.isEmpty)
            Button("Zoom In ") { model.canvas?.zoomIn() }.keyboardShortcut("=").disabled(model.isEmpty)
            Button("Zoom Out") { model.canvas?.zoomOut() }.keyboardShortcut("-").disabled(model.isEmpty)
            Button("Fit in Window") { model.canvas?.zoomToFit() }.keyboardShortcut("0").disabled(model.isEmpty)
            Button("Actual Size") { model.canvas?.setZoom(1) }.keyboardShortcut("1").disabled(model.isEmpty)
            Divider()
            Picker("Appearance", selection: $appearance) {
                ForEach(AppearancePreference.allCases) { a in Text(a.title).tag(a.rawValue) }
            }
            Divider()
            Button(showTools ? "Hide Tools" : "Show Tools") { withAnimation(.snappy) { showTools.toggle() } }.keyboardShortcut("t", modifiers: [.command, .option])
            Button(showLayers ? "Hide Layers" : "Show Layers") { withAnimation(.snappy) { showLayers.toggle() } }.keyboardShortcut("l", modifiers: [.command, .option])
        }
    }
}

/// Startup measurement: with CUTOUT_STARTUP_PROBE=<file>, writes the time from
/// process creation to the main window appearing (ms), then quits.
enum StartupProbe {
    static func windowAppeared() {
        guard let path = ProcessInfo.processInfo.environment["CUTOUT_STARTUP_PROBE"], !path.isEmpty else { return }
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        sysctl(&mib, 4, &info, &size, nil, 0)
        let st = info.kp_proc.p_starttime
        let start = Double(st.tv_sec) + Double(st.tv_usec) / 1e6
        // wait for the first frame to be committed
        DispatchQueue.main.async {
            let ms = (Date().timeIntervalSince1970 - start) * 1000
            var mi = mach_task_basic_info()
            var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size) / 4
            _ = withUnsafeMutablePointer(to: &mi) {
                $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count) }
            }
            try? String(format: "{\"startup_ms\": %.0f, \"resident_mb\": %d}\n", ms, Int(mi.resident_size / 1_048_576)).write(toFile: path, atomically: true, encoding: .utf8)
            NSApp.terminate(nil)
        }
    }
}
