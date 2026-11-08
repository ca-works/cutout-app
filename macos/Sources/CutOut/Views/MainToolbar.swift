import SwiftUI

struct MainToolbar: ToolbarContent {
    @Environment(EditorModel.self) private var model
    @Binding var showTools: Bool
    @Binding var showLayers: Bool

    var body: some ToolbarContent {
        ToolbarItemGroup(placement: .navigation) {
            Button {
                model.openPanel()
            } label: {
                Label("Open", systemImage: "folder")
            }
            .help("Open an image or project (⌘O)")
        }

        ToolbarItemGroup(placement: .principal) {
            if !model.isEmpty {
                Button {
                    model.removeBackground(.best)
                } label: {
                    Label("Remove Background", systemImage: "wand.and.sparkles")
                        .labelStyle(.titleAndIcon)
                        .padding(.horizontal, 4)
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.isRemoving)
                .help("Remove the background with the best-quality local AI model (⌘B)")
                .contextMenu {
                    Button("Best Quality") { model.removeBackground(.best) }
                    Button("Fast (Apple Vision)") { model.removeBackground(.fast) }
                }

                Menu {
                    Button("Best Quality (BiRefNet)") { model.removeBackground(.best) }
                    Button("Fast (Apple Vision)") { model.removeBackground(.fast) }
                    Divider()
                    Button("Refine Edges…") {
                        showTools = true
                        model.panel = .refine
                    }
                    Button("Manage AI Models…") { model.sheet = .modelSetup }
                } label: {
                    Label("Removal Options", systemImage: "chevron.down")
                }
                .menuIndicator(.hidden)
                .fixedSize()
                .help("Removal mode and edge refinement")

                Toggle(isOn: Binding(get: { model.compareOn }, set: { model.compareOn = $0 })) {
                    Label("Compare", systemImage: "rectangle.split.2x1")
                }
                .help("Compare before and after (\\)")
            }
        }

        ToolbarItemGroup(placement: .primaryAction) {
            if !model.isEmpty {
                ControlGroup {
                    Button { model.undo() } label: { Label("Undo", systemImage: "arrow.uturn.backward") }
                        .disabled(!model.state.canUndo)
                        .help(model.state.undoLabel.map { "Undo \($0) (⌘Z)" } ?? "Undo (⌘Z)")
                    Button { model.redo() } label: { Label("Redo", systemImage: "arrow.uturn.forward") }
                        .disabled(!model.state.canRedo)
                        .help(model.state.redoLabel.map { "Redo \($0) (⇧⌘Z)" } ?? "Redo (⇧⌘Z)")
                }

                // Clipboard-first: one primary Copy, with Clear / Close variants one click away.
                Menu {
                    Button("Copy") { model.copy(.copy) }
                    Button("Copy & Clear") { model.copy(.copyAndClear) }
                    Button("Copy & Close") { model.copy(.copyAndClose) }
                    Divider()
                    Button("Copy as PNG") { model.copy(.copy, format: .png) }
                    Button("Copy as JPEG") { model.copy(.copy, format: .jpeg) }
                } label: {
                    Label("Copy", systemImage: "doc.on.doc").labelStyle(.titleAndIcon)
                } primaryAction: {
                    model.copy(.copy)
                }
                .help("Copy the result with transparency (⌘C). Menu: Copy & Clear (⇧⌘C), Copy & Close (⌥⌘C)")
            }

            Toggle(isOn: $showTools.animation(.smooth(duration: 0.28))) {
                Label("Advanced Tools", systemImage: "slider.horizontal.3")
            }
            .disabled(model.isEmpty)
            .help("Show or hide the advanced tools (⌥⌘T)")

            Toggle(isOn: $showLayers.animation(.smooth(duration: 0.28))) {
                Label("Layers", systemImage: "square.3.layers.3d")
            }
            .disabled(model.isEmpty)
            .help("Show or hide the Layers panel (⌥⌘L)")
        }
    }
}
