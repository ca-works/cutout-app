import SwiftUI

struct ToolSidebar: View {
    @Environment(EditorModel.self) private var model
    @Namespace private var selectionPill

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                ForEach(Tool.groups, id: \.0) { group in
                    VStack(alignment: .leading, spacing: 6) {
                        SectionHeader(group.0)
                        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 6), count: 4), spacing: 6) {
                            ForEach(group.1) { t in ToolButton(tool: t, pill: selectionPill) }
                        }
                    }
                }
                ToolOptions()
                    .id(model.tool)
                    .transition(.opacity)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .cardSurface()
                ActionsSection()
            }
            .padding(12)
            .motion(.snappy(duration: 0.2), value: model.tool)
        }
        .scrollIndicators(.automatic)
    }
}

struct SectionHeader: View {
    let title: String
    init(_ t: String) { title = t }
    var body: some View {
        Text(title.uppercased())
            .font(.caption2.weight(.semibold))
            .foregroundStyle(.secondary)
            .tracking(0.6)
    }
}

struct ToolButton: View {
    @Environment(EditorModel.self) private var model
    let tool: Tool
    var pill: Namespace.ID
    @State private var hover = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let selected = model.tool == tool
        Button {
            withAnimation(reduceMotion ? nil : .snappy(duration: 0.22)) { model.tool = tool }
        } label: {
            Image(systemName: tool.symbol)
                .font(.system(size: 15, weight: .regular))
                .frame(maxWidth: .infinity, minHeight: 36)
                .foregroundStyle(selected ? Color.white : Color.primary)
                .background {
                    ZStack {
                        if hover && !selected {
                            RoundedRectangle(cornerRadius: 9, style: .continuous).fill(Theme.hover)
                        }
                        if selected {
                            RoundedRectangle(cornerRadius: 9, style: .continuous)
                                .fill(LinearGradient(colors: [Color.accentColor, Color.accentColor.opacity(0.82)], startPoint: .top, endPoint: .bottom))
                                .shadow(color: Color.accentColor.opacity(0.35), radius: 4, y: 2)
                                .matchedGeometryEffect(id: "pill", in: pill)
                        }
                    }
                }
                .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
        .buttonStyle(PressableStyle())
        .background {
            GeometryReader { g in
                Color.clear.onChange(of: g.frame(in: .global), initial: true) { _, f in if HitTest.enabled { HitTest.frames[tool] = f } }
            }
        }
        .onHover { hover = $0 }
        .help("\(tool.title) (\(String(tool.shortcut).uppercased()))")
        .accessibilityLabel(tool.title)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .animation(.easeOut(duration: 0.12), value: hover)
    }
}

struct PressableStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.94 : 1)
            .animation(.snappy(duration: 0.12), value: configuration.isPressed)
    }
}

/// A labelled slider row with a numeric readout.
struct SliderRow: View {
    let title: String
    @Binding var value: Double
    var range: ClosedRange<Double>
    var step: Double? = nil
    var format: (Double) -> String = { String(Int($0.rounded())) }
    var onEditingChanged: (Bool) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(title)
                Spacer()
                Text(format(value)).monospacedDigit().foregroundStyle(.secondary)
            }
            .font(.callout)
            if let step {
                Slider(value: $value, in: range, step: step, onEditingChanged: onEditingChanged)
                    .controlSize(.small)
            } else {
                Slider(value: $value, in: range, onEditingChanged: onEditingChanged)
                    .controlSize(.small)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(title)
        .accessibilityValue(format(value))
    }
}

let pct: (Double) -> String = { "\(Int(($0 * 100).rounded()))%" }
let px: (Double) -> String = { "\(Int($0.rounded())) px" }

struct ToolOptions: View {
    @Environment(EditorModel.self) private var model

    var body: some View {
        @Bindable var model = model
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label(model.tool.title, systemImage: model.tool.symbol).font(.headline)
                Spacer()
                KeyCap(String(model.tool.shortcut).uppercased())
            }
            Text(model.tool.help).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            switch model.tool {
            case .rectSelect, .ellipseSelect, .lasso, .polygonLasso:
                SelectionModePicker()
                SliderRow(title: "Feather", value: $model.settings.feather, range: 0...50, format: px)
            case .magicWand:
                SelectionModePicker()
                colorTolerance
                Toggle("Sample all layers", isOn: $model.settings.sampleAll)
                SliderRow(title: "Feather", value: $model.settings.feather, range: 0...50, format: px)
            case .quickSelect:
                SliderRow(title: "Brush size", value: $model.settings.quickSize, range: 2...400, format: px)
                SliderRow(title: "Edge sensitivity", value: $model.settings.quickEdge, range: 0...0.95, format: pct)
                Text("Hold Option to subtract. Use [ and ] to resize.").font(.caption).foregroundStyle(.secondary)
            case .magicEraser:
                colorTolerance
                SliderRow(title: "Edge feather", value: $model.settings.eraseFeather, range: 0...20, format: px)
            case .eraser:
                SliderRow(title: "Size", value: $model.settings.eraserSize, range: 1...800, format: px)
                SliderRow(title: "Hardness", value: $model.settings.eraserHardness, range: 0...1, format: pct)
                SliderRow(title: "Strength", value: $model.settings.eraserStrength, range: 0.05...1, format: pct)
            case .restore:
                SliderRow(title: "Size", value: $model.settings.restoreSize, range: 1...800, format: px)
                SliderRow(title: "Hardness", value: $model.settings.restoreHardness, range: 0...1, format: pct)
                SliderRow(title: "Restore strength", value: $model.settings.restoreStrength, range: 0.05...1, format: pct)
            case .brush:
                ColorRow(title: "Colour", rgba: $model.settings.color)
                SliderRow(title: "Size", value: $model.settings.brushSize, range: 1...800, format: px)
                SliderRow(title: "Hardness", value: $model.settings.brushHardness, range: 0...1, format: pct)
                SliderRow(title: "Opacity", value: $model.settings.brushOpacity, range: 0.05...1, format: pct)
            case .fill:
                ColorRow(title: "Colour", rgba: $model.settings.color)
                colorTolerance
                SliderRow(title: "Opacity", value: $model.settings.fillOpacity, range: 0.05...1, format: pct)
                Button("Fill Selection") {
                    model.run("fill", ["selection": true, "color": model.settings.colorRGBA, "opacity": model.settings.fillOpacity])
                }
                .disabled(!model.state.hasSelection)
            case .eyedropper:
                ColorRow(title: "Current colour", rgba: $model.settings.color)
            case .text:
                ColorRow(title: "Colour", rgba: $model.settings.color)
                FontPicker(name: $model.settings.fontName)
                SliderRow(title: "Size", value: $model.settings.fontSize, range: 8...600, format: { "\(Int($0)) pt" })
                HStack {
                    Toggle("Bold", isOn: $model.settings.bold)
                    Toggle("Italic", isOn: $model.settings.italic)
                }
                .toggleStyle(.button)
                if let l = model.state.activeLayer, l.isText {
                    Button("Edit Selected Text…") { model.sheet = .text(layerID: l.id, point: .zero) }
                }
            case .shape:
                Picker("Shape", selection: $model.settings.shapeKind) {
                    ForEach(ShapeKind.allCases) { k in Image(systemName: k.symbol).tag(k.rawValue) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                ColorRow(title: "Stroke", rgba: $model.settings.color)
                SliderRow(title: "Stroke width", value: $model.settings.shapeStroke, range: 0...80, format: px)
                Toggle("Fill", isOn: $model.settings.shapeFilled)
                if model.settings.shapeFilled { ColorRow(title: "Fill colour", rgba: $model.settings.fillColor) }
                if model.settings.shapeKind == "rect" {
                    SliderRow(title: "Corner radius", value: $model.settings.shapeCorner, range: 0...200, format: px)
                }
            case .crop:
                HStack {
                    Button("Apply Crop") { model.canvas?.applyCrop() }
                        .buttonStyle(.borderedProminent)
                        .keyboardShortcut(.return, modifiers: [])
                    Button("Trim Transparent") { model.run("trim"); model.canvas?.zoomToFit() }
                }
            case .move:
                if let l = model.state.activeLayer {
                    TransformFields(layer: l)
                }
            case .hand, .zoom:
                HStack {
                    Button("Fit") { model.canvas?.zoomToFit() }
                    Button("100%") { model.canvas?.setZoom(1) }
                }
            }
        }
    }

    @ViewBuilder private var colorTolerance: some View {
        @Bindable var model = model
        SliderRow(title: "Tolerance (ΔE)", value: $model.settings.tolerance, range: 0...100, format: { String(format: "%.0f", $0) })
        Toggle("Contiguous", isOn: $model.settings.contiguous)
            .help("Only affect connected pixels. Turn off to affect the colour everywhere in the layer.")
        Toggle("Anti-alias edges", isOn: $model.settings.antialias)
    }
}

struct SelectionModePicker: View {
    @Environment(EditorModel.self) private var model
    var body: some View {
        @Bindable var model = model
        Picker("Mode", selection: $model.selectionMode) {
            ForEach(SelectionMode.allCases) { m in
                Image(systemName: m.symbol).help(m.title).tag(m)
            }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .accessibilityLabel("Selection mode")
    }
}

struct ColorRow: View {
    let title: String
    @Binding var rgba: [Double]
    var body: some View {
        ColorPicker(title, selection: Binding(get: { Color(rgba: rgba) }, set: { rgba = $0.rgbaComponents }), supportsOpacity: true)
            .font(.callout)
    }
}

struct FontPicker: View {
    @Binding var name: String
    private static let families: [String] = NSFontManager.shared.availableFontFamilies

    var body: some View {
        Picker("Font", selection: $name) {
            ForEach(Self.families, id: \.self) { f in Text(f).tag(f) }
        }
        .font(.callout)
    }
}

struct TransformFields: View {
    @Environment(EditorModel.self) private var model
    let layer: LayerInfo

    var body: some View {
        let t = layer.transform
        VStack(alignment: .leading, spacing: 6) {
            Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 4) {
                GridRow {
                    Text("Position").foregroundStyle(.secondary)
                    Text("\(Int(t.x - Double(layer.width) * t.scaleX / 2)), \(Int(t.y - Double(layer.height) * t.scaleY / 2))").monospacedDigit()
                }
                GridRow {
                    Text("Scale").foregroundStyle(.secondary)
                    Text(String(format: "%.0f%%", t.scaleX * 100)).monospacedDigit()
                }
                GridRow {
                    Text("Rotation").foregroundStyle(.secondary)
                    Text(String(format: "%.1f°", t.rotation * 180 / .pi)).monospacedDigit()
                }
            }
            .font(.callout)
            HStack {
                Button("Reset") { model.run("reset_transform") }
                Button { model.run("flip_layer", ["axis": "h"]) } label: { Image(systemName: "arrow.left.and.right.righttriangle.left.righttriangle.right") }
                    .help("Flip layer horizontally")
                Button { model.run("flip_layer", ["axis": "v"]) } label: { Image(systemName: "arrow.up.and.down.righttriangle.up.righttriangle.down") }
                    .help("Flip layer vertically")
                Button {
                    var nt = t
                    nt.rotation += .pi / 2
                    model.run("set_transform", ["id": layer.id, "transform": nt.json, "label": "Rotate Layer"])
                } label: { Image(systemName: "rotate.right") }
                    .help("Rotate layer 90°")
            }
            Text("Shift constrains movement and snaps rotation to 15°. Arrow keys nudge.").font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// Contextual actions grouped into collapsible sections.
struct ActionsSection: View {
    @Environment(EditorModel.self) private var model
    @AppStorage("sec.background") private var bgOpen = true
    @AppStorage("sec.selection") private var selOpen = true
    @AppStorage("sec.image") private var imgOpen = false
    @AppStorage("sec.adjust") private var adjOpen = false

    var body: some View {
        @Bindable var model = model
        VStack(alignment: .leading, spacing: 8) {
            DisclosureGroup(isExpanded: Binding(get: { bgOpen || model.panel == .refine }, set: { bgOpen = $0; if !$0 && model.panel == .refine { model.panel = .none } })) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Button("Best Quality") { model.removeBackground(.best) }
                            .buttonStyle(.borderedProminent)
                        Button("Fast") { model.removeBackground(.fast) }
                    }
                    .disabled(model.isRemoving)
                    RefineEdgesPanel()
                    HStack {
                        Button("Restore All") { model.run("clear_mask") }
                            .help("Bring back every erased or removed pixel on this layer")
                        Button("Invert Mask") { model.run("invert_mask") }
                    }
                    .controlSize(.small)
                }
                .padding(.top, 6)
            } label: { SectionHeader("Background & Edges") }

            DisclosureGroup(isExpanded: $selOpen) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Button("All") { model.run("select_all") }
                        Button("None") { model.run("deselect") }.disabled(!model.state.hasSelection)
                        Button("Invert") { model.run("invert_selection") }
                    }
                    HStack {
                        Button("Delete Pixels") { model.run("delete_selection") }
                        Button("Keep Only") { model.run("mask_from_selection") }
                    }
                    .disabled(!model.state.hasSelection)
                    HStack {
                        Button("Grow") { model.run("grow_selection", ["amount": 3]) }
                        Button("Shrink") { model.run("grow_selection", ["amount": -3]) }
                        Button("Smooth") { model.run("smooth_selection", ["amount": 2]) }
                        Button("Feather") { model.run("feather_selection", ["amount": 3]) }
                    }
                    .disabled(!model.state.hasSelection)
                    Button("Crop to Selection") { model.run("crop_to_selection"); model.canvas?.zoomToFit() }
                        .disabled(!model.state.hasSelection)
                }
                .controlSize(.small)
                .padding(.top, 6)
            } label: { SectionHeader("Selection") }

            DisclosureGroup(isExpanded: $imgOpen) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Button("Resize…") { model.sheet = .resize }
                        Button("Canvas…") { model.sheet = .canvasSize }
                        Button("Trim") { model.run("trim"); model.canvas?.zoomToFit() }
                    }
                    HStack {
                        Button { model.run("rotate_canvas", ["direction": "ccw"]); model.canvas?.zoomToFit() } label: { Image(systemName: "rotate.left") }.help("Rotate 90° left")
                        Button { model.run("rotate_canvas", ["direction": "cw"]); model.canvas?.zoomToFit() } label: { Image(systemName: "rotate.right") }.help("Rotate 90° right")
                        Button { model.run("flip_canvas", ["axis": "h"]) } label: { Image(systemName: "arrow.left.and.right") }.help("Flip horizontal")
                        Button { model.run("flip_canvas", ["axis": "v"]) } label: { Image(systemName: "arrow.up.and.down") }.help("Flip vertical")
                    }
                }
                .controlSize(.small)
                .padding(.top, 6)
            } label: { SectionHeader("Image") }

            DisclosureGroup(isExpanded: Binding(get: { adjOpen || model.panel == .adjust }, set: { adjOpen = $0; if !$0 && model.panel == .adjust { model.panel = .none } })) {
                AdjustmentsPanel().padding(.top, 6)
            } label: { SectionHeader("Adjustments") }
        }
    }
}

struct RefineEdgesPanel: View {
    @Environment(EditorModel.self) private var model
    @State private var radius: Double = 0
    @State private var smooth: Double = 0
    @State private var feather: Double = 0
    @State private var contrast: Double = 0
    @State private var shift: Double = 0
    @State private var decontaminate = false
    @State private var active = false

    private var params: [String: Any] {
        ["radius": radius, "smooth": smooth, "feather": feather, "contrast": contrast, "shift": shift, "decontaminate": decontaminate]
    }

    private var isIdentity: Bool { radius == 0 && smooth == 0 && feather == 0 && contrast == 0 && shift == 0 && !decontaminate }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Refine Edges").font(.callout.weight(.semibold))
            SliderRow(title: "Edge detection radius", value: $radius, range: 0...30, format: px, onEditingChanged: { if !$0 { preview() } })
            SliderRow(title: "Smooth", value: $smooth, range: 0...10, format: { String(format: "%.1f", $0) }, onEditingChanged: { if !$0 { preview() } })
            SliderRow(title: "Feather", value: $feather, range: 0...20, format: { String(format: "%.1f px", $0) }, onEditingChanged: { if !$0 { preview() } })
            SliderRow(title: "Contrast", value: $contrast, range: 0...1, format: pct, onEditingChanged: { if !$0 { preview() } })
            SliderRow(title: "Shift edge", value: $shift, range: -20...20, step: 1, format: { "\(Int($0)) px" }, onEditingChanged: { if !$0 { preview() } })
            Toggle("Decontaminate colours", isOn: $decontaminate)
                .onChange(of: decontaminate) { preview() }
                .help("Removes background colour fringing from semi-transparent edges")
            HStack {
                Button("Apply") {
                    model.run("refine_commit", params)
                    reset()
                }
                .buttonStyle(.borderedProminent)
                .disabled(isIdentity)
                Button("Cancel") {
                    model.run("preview_cancel")
                    reset()
                }
                .disabled(!active)
            }
            .controlSize(.small)
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(.quaternary.opacity(0.5)))
    }

    private func preview() {
        guard !isIdentity else {
            model.run("preview_cancel")
            active = false
            return
        }
        active = true
        model.run("refine_preview", params)
    }

    private func reset() {
        radius = 0; smooth = 0; feather = 0; contrast = 0; shift = 0; decontaminate = false; active = false
    }
}

struct AdjustmentsPanel: View {
    @Environment(EditorModel.self) private var model
    @State private var a: [String: Double] = [:]
    @State private var invert = false
    @State private var gray = false

    private let rows: [(String, String, ClosedRange<Double>)] = [
        ("exposure", "Exposure", -3...3), ("brightness", "Brightness", -1...1), ("contrast", "Contrast", -1...1),
        ("highlights", "Highlights", -1...1), ("shadows", "Shadows", -1...1), ("saturation", "Saturation", -1...1),
        ("temperature", "Temperature", -1...1), ("tint", "Tint", -1...1), ("hue", "Hue", -180...180), ("sharpen", "Sharpen", 0...1),
    ]

    private var payload: [String: Any] {
        var d: [String: Any] = a
        d["invert"] = invert
        d["grayscale"] = gray
        return ["adjustments": d]
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(rows, id: \.0) { r in
                SliderRow(title: r.1, value: Binding(get: { a[r.0] ?? 0 }, set: { a[r.0] = $0 }), range: r.2,
                          format: { r.0 == "hue" ? "\(Int($0))°" : String(format: "%+.2f", $0) },
                          onEditingChanged: { if !$0 { model.run("adjust_preview", payload) } })
            }
            HStack {
                Toggle("Invert", isOn: $invert)
                Toggle("Greyscale", isOn: $gray)
            }
            .onChange(of: invert) { model.run("adjust_preview", payload) }
            .onChange(of: gray) { model.run("adjust_preview", payload) }
            HStack {
                Button("Apply") {
                    model.run("adjust_commit", payload)
                    a = [:]; invert = false; gray = false
                }
                .buttonStyle(.borderedProminent)
                Button("Reset") {
                    model.run("preview_cancel")
                    a = [:]; invert = false; gray = false
                }
                Button("Auto Levels") { model.run("auto_levels") }
            }
            .controlSize(.small)
        }
    }
}
