import SwiftUI

enum Tool: String, CaseIterable, Identifiable, Codable {
    case move, rectSelect, ellipseSelect, lasso, polygonLasso, magicWand, quickSelect
    case magicEraser, eraser, restore, brush, fill, eyedropper
    case text, shape, crop, hand, zoom

    var id: String { rawValue }

    var title: String {
        switch self {
        case .move: "Move & Transform"
        case .rectSelect: "Rectangle Select"
        case .ellipseSelect: "Ellipse Select"
        case .lasso: "Lasso"
        case .polygonLasso: "Polygon Lasso"
        case .magicWand: "Magic Wand"
        case .quickSelect: "Quick Selection"
        case .magicEraser: "Magic Eraser"
        case .eraser: "Eraser"
        case .restore: "Restore Brush"
        case .brush: "Brush"
        case .fill: "Paint Bucket"
        case .eyedropper: "Eyedropper"
        case .text: "Text"
        case .shape: "Shapes"
        case .crop: "Crop"
        case .hand: "Hand"
        case .zoom: "Zoom"
        }
    }

    var symbol: String {
        switch self {
        case .move: "arrow.up.and.down.and.arrow.left.and.right"
        case .rectSelect: "rectangle.dashed"
        case .ellipseSelect: "circle.dashed"
        case .lasso: "lasso"
        case .polygonLasso: "skew"
        case .magicWand: "wand.and.rays"
        case .quickSelect: "paintbrush.pointed"
        case .magicEraser: "wand.and.stars.inverse"
        case .eraser: "eraser"
        case .restore: "arrow.uturn.backward.circle"
        case .brush: "paintbrush"
        case .fill: "drop"
        case .eyedropper: "eyedropper"
        case .text: "textformat"
        case .shape: "square.on.circle"
        case .crop: "crop"
        case .hand: "hand.raised"
        case .zoom: "plus.magnifyingglass"
        }
    }

    var shortcut: Character {
        switch self {
        case .move: "v"
        case .rectSelect: "m"
        case .ellipseSelect: "o"
        case .lasso: "l"
        case .polygonLasso: "p"
        case .magicWand: "w"
        case .quickSelect: "q"
        case .magicEraser: "e"
        case .eraser: "x"
        case .restore: "r"
        case .brush: "b"
        case .fill: "g"
        case .eyedropper: "i"
        case .text: "t"
        case .shape: "u"
        case .crop: "c"
        case .hand: "h"
        case .zoom: "z"
        }
    }

    var help: String {
        switch self {
        case .move: "Drag to move the layer. Drag corners to scale, the top handle to rotate. Click a layer's pixels to select it."
        case .rectSelect, .ellipseSelect: "Drag to select. Shift adds, Option subtracts."
        case .lasso: "Drag around an area to select it. Shift adds, Option subtracts."
        case .polygonLasso: "Click to add points; double-click or press Return to close. Esc cancels."
        case .magicWand: "Click a colour to select similar connected pixels (ΔE colour distance)."
        case .quickSelect: "Paint over an object; the selection grows to its edges. Option subtracts."
        case .magicEraser: "Click a colour to erase it. Respects anti-aliased edges; erased pixels can be restored."
        case .eraser: "Paint to hide pixels (non-destructive)."
        case .restore: "Paint to bring back erased or removed pixels."
        case .brush: "Paint with the current colour."
        case .fill: "Click to fill an area of similar colour."
        case .eyedropper: "Click to pick a colour."
        case .text: "Click to add text."
        case .shape: "Drag to draw a shape on a new layer."
        case .crop: "Drag the crop area, then press Return."
        case .hand: "Drag to pan. Hold Space with any tool."
        case .zoom: "Click to zoom in, Option-click to zoom out."
        }
    }

    var isBrush: Bool { [.eraser, .restore, .brush, .quickSelect].contains(self) }

    static let groups: [(String, [Tool])] = [
        ("Select", [.move, .rectSelect, .ellipseSelect, .lasso, .polygonLasso, .magicWand, .quickSelect]),
        ("Retouch", [.magicEraser, .eraser, .restore, .brush, .fill, .eyedropper]),
        ("Create", [.text, .shape, .crop, .hand, .zoom]),
    ]
}

enum SelectionMode: String, CaseIterable, Identifiable {
    case replace, add, subtract, intersect
    var id: String { rawValue }
    var symbol: String {
        switch self {
        case .replace: "square"
        case .add: "plus.square"
        case .subtract: "minus.square"
        case .intersect: "square.on.square.intersection.dashed"
        }
    }
    var title: String { rawValue.capitalized }
}

enum ShapeKind: String, CaseIterable, Identifiable {
    case rect, ellipse, line, arrow
    var id: String { rawValue }
    var symbol: String {
        switch self {
        case .rect: "rectangle"
        case .ellipse: "circle"
        case .line: "line.diagonal"
        case .arrow: "arrow.up.right"
        }
    }
}

/// Per-tool settings, persisted between sessions.
struct ToolSettings: Codable, Equatable {
    var brushSize: Double = 40
    var brushHardness: Double = 0.8
    var brushOpacity: Double = 1
    var eraserSize: Double = 40
    var eraserHardness: Double = 0.7
    var restoreSize: Double = 40
    var restoreHardness: Double = 0.7
    var restoreStrength: Double = 1
    var eraserStrength: Double = 1
    var tolerance: Double = 10
    var contiguous: Bool = true
    var antialias: Bool = true
    var sampleAll: Bool = false
    var feather: Double = 0
    var eraseFeather: Double = 0
    var quickSize: Double = 30
    var quickEdge: Double = 0.5
    var fillOpacity: Double = 1
    var shapeKind: String = ShapeKind.rect.rawValue
    var shapeStroke: Double = 6
    var shapeFilled: Bool = false
    var shapeCorner: Double = 0
    var fontName: String = "Helvetica Neue"
    var fontSize: Double = 72
    var bold: Bool = true
    var italic: Bool = false
    var color: [Double] = [0.0, 0.48, 1.0, 1.0]
    var fillColor: [Double] = [1, 1, 1, 1]

    var colorRGBA: [Int] { color.map { Int(($0 * 255).rounded()) } }
    var fillRGBA: [Int] { fillColor.map { Int(($0 * 255).rounded()) } }

    func size(for tool: Tool) -> Double {
        switch tool {
        case .eraser: eraserSize
        case .restore: restoreSize
        case .quickSelect: quickSize
        default: brushSize
        }
    }

    mutating func setSize(_ v: Double, for tool: Tool) {
        let v = min(max(v, 1), 2000)
        switch tool {
        case .eraser: eraserSize = v
        case .restore: restoreSize = v
        case .quickSelect: quickSize = v
        default: brushSize = v
        }
    }
}

extension Color {
    init(rgba: [Double]) {
        self.init(.sRGB, red: rgba[0], green: rgba[1], blue: rgba[2], opacity: rgba.count > 3 ? rgba[3] : 1)
    }

    var rgbaComponents: [Double] {
        let ns = NSColor(self).usingColorSpace(.sRGB) ?? .black
        return [ns.redComponent, ns.greenComponent, ns.blueComponent, ns.alphaComponent].map { Double($0) }
    }
}
