import SwiftUI

struct LayersPanel: View {
    @Environment(EditorModel.self) private var model
    @State private var renaming: Int64?
    @State private var nameDraft = ""
    @State private var opacityEditing = false

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Label("Layers", systemImage: "square.3.layers.3d").font(.headline)
                Spacer()
                Text("\(model.state.layers.count)").foregroundStyle(.secondary).monospacedDigit()
            }
            .padding(.horizontal, 12).padding(.vertical, 8)

            if let active = model.state.activeLayer {
                HStack(spacing: 8) {
                    Text("Opacity").font(.callout).foregroundStyle(.secondary)
                    Slider(value: Binding(get: { active.opacity }, set: { v in
                        model.setOpacity(active.id, v, begin: !opacityEditing)
                        opacityEditing = true
                    }), in: 0...1, onEditingChanged: { if !$0 { opacityEditing = false } })
                    .controlSize(.small)
                    Text("\(Int((active.opacity * 100).rounded()))%").font(.callout).monospacedDigit().frame(width: 38, alignment: .trailing)
                }
                .padding(.horizontal, 12).padding(.bottom, 6)
            }

            List(selection: Binding<Set<Int64>>(
                get: { model.selectedLayerIDs.isEmpty ? Set([model.state.active].compactMap { $0 }) : model.selectedLayerIDs },
                set: { ids in
                    model.selectedLayerIDs = ids
                    if let first = ids.first, ids.count == 1 { model.run("select_layer", ["id": first]) }
                    else if let a = model.state.active, !ids.contains(a), let f = ids.first { model.run("select_layer", ["id": f]) }
                })) {
                ForEach(model.state.layers.reversed()) { layer in
                    row(layer)
                        .tag(layer.id)
                        .contextMenu { menu(layer) }
                }
                .onMove { from, to in
                    // List shows top layer first; engine stores bottom first.
                    let n = model.state.layers.count
                    guard let f = from.first else { return }
                    let srcIdx = n - 1 - f
                    let d = to > f ? to - 1 : to
                    let dst = n - 1 - d
                    let id = model.state.layers[srcIdx].id
                    model.run("move_layer", ["id": id, "index": max(0, min(n - 1, dst))])
                }
            }
            .listStyle(.sidebar)
            .scrollContentBackground(.hidden)
            .motion(.snappy(duration: 0.22), value: model.state.layers.map(\.id))

            Divider()
            HStack(spacing: 2) {
                iconButton("plus.square.on.square", "Paste as New Layer (⌘V)") { model.paste() }
                iconButton("square.on.square", "Duplicate Layer (⌘J)") { model.run("duplicate_layer") }
                iconButton("arrow.down.to.line", "Merge Down / Merge Selected (⌘E)") { model.mergeSelectedOrDown() }
                iconButton("square.stack.3d.down.forward", "Flatten Image") { model.run("flatten") }
                Spacer()
                iconButton("chevron.up", "Move Layer Up") { if let a = model.state.active { model.run("move_layer", ["id": a, "direction": "up"]) } }
                iconButton("chevron.down", "Move Layer Down") { if let a = model.state.active { model.run("move_layer", ["id": a, "direction": "down"]) } }
                iconButton("trash", "Delete Layer") { model.run("delete_layer") }
                    .disabled(model.state.layers.count <= 1)
            }
            .padding(6)
        }
    }

    private func iconButton(_ symbol: String, _ help: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol).frame(width: 26, height: 24).contentShape(Rectangle())
        }
        .buttonStyle(SoftButtonStyle())
        .help(help)
        .accessibilityLabel(help)
    }

    @ViewBuilder
    private func row(_ layer: LayerInfo) -> some View {
        HStack(spacing: 8) {
            Button {
                model.run("set_layer", ["id": layer.id, "visible": !layer.visible])
            } label: {
                Image(systemName: layer.visible ? "eye" : "eye.slash")
                    .foregroundStyle(layer.visible ? .secondary : .tertiary)
                    .frame(width: 18)
            }
            .buttonStyle(.borderless)
            .help(layer.visible ? "Hide layer" : "Show layer")
            .accessibilityLabel(layer.visible ? "Hide \(layer.name)" : "Show \(layer.name)")

            ZStack {
                CheckerSwatch()
                if let img = model.thumbnail(for: layer) {
                    Image(nsImage: img).resizable().interpolation(.medium).aspectRatio(contentMode: .fit)
                }
            }
            .frame(width: 38, height: 38)
            .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous).strokeBorder(Theme.cardStroke, lineWidth: 1))

            VStack(alignment: .leading, spacing: 1) {
                if renaming == layer.id {
                    TextField("Name", text: $nameDraft)
                        .textFieldStyle(.plain)
                        .onSubmit { commitRename(layer.id) }
                        .onExitCommand { renaming = nil }
                } else {
                    Text(layer.name).lineLimit(1)
                        .onTapGesture(count: 2) { nameDraft = layer.name; renaming = layer.id }
                }
                HStack(spacing: 4) {
                    if layer.isText { Image(systemName: "textformat").font(.caption2) }
                    if layer.hasMask { Image(systemName: "circle.lefthalf.filled").font(.caption2).help("Has a mask — erased pixels can be restored") }
                    if layer.opacity < 1 { Text("\(Int(layer.opacity * 100))%").font(.caption2) }
                }
                .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            if layer.locked { Image(systemName: "lock.fill").font(.caption).foregroundStyle(.secondary) }
        }
        .padding(.vertical, 2)
        .opacity(layer.visible ? 1 : 0.55)
    }

    private func commitRename(_ id: Int64) {
        model.run("set_layer", ["id": id, "name": nameDraft])
        renaming = nil
    }

    @ViewBuilder
    private func menu(_ layer: LayerInfo) -> some View {
        Button("Rename") { nameDraft = layer.name; renaming = layer.id }
        Button("Duplicate") { model.run("duplicate_layer", ["id": layer.id]) }
        Button(layer.visible ? "Hide" : "Show") { model.run("set_layer", ["id": layer.id, "visible": !layer.visible]) }
        Button(layer.locked ? "Unlock" : "Lock") { model.run("set_layer", ["id": layer.id, "locked": !layer.locked]) }
        Divider()
        Button("Merge Down") { model.run("merge_down", ["id": layer.id]) }
        if model.selectedLayerIDs.count >= 2 { Button("Merge Selected Layers") { model.mergeSelectedOrDown() } }
        Button("Flatten Image") { model.run("flatten") }
        Divider()
        if layer.hasMask {
            Button("Apply Mask") { model.run("select_layer", ["id": layer.id]); model.run("apply_mask") }
            Button("Restore All Pixels") { model.run("select_layer", ["id": layer.id]); model.run("clear_mask") }
        }
        if layer.isText {
            Button("Edit Text…") { model.sheet = .text(layerID: layer.id, point: .zero) }
            Button("Rasterize") { model.run("select_layer", ["id": layer.id]); model.run("rasterize_layer") }
        }
        Divider()
        Button("Delete", role: .destructive) { model.run("delete_layer", ["id": layer.id]) }
            .disabled(model.state.layers.count <= 1)
    }
}

struct CheckerSwatch: View {
    var body: some View {
        Canvas { ctx, size in
            let c: CGFloat = 5
            ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Color(white: 0.92)))
            var y: CGFloat = 0
            var row = 0
            while y < size.height {
                var x: CGFloat = row % 2 == 0 ? 0 : c
                while x < size.width {
                    ctx.fill(Path(CGRect(x: x, y: y, width: c, height: c)), with: .color(Color(white: 0.78)))
                    x += c * 2
                }
                y += c
                row += 1
            }
        }
    }
}
