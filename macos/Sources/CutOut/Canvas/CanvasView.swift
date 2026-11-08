import AppKit
import CCutOut
import QuartzCore
import SwiftUI

/// SwiftUI wrapper.
struct CanvasView: NSViewRepresentable {
    let model: EditorModel

    func makeNSView(context: Context) -> CanvasNSView {
        let v = CanvasNSView(model: model)
        model.canvas = v
        return v
    }

    func updateNSView(_ v: CanvasNSView, context: Context) {}
}

/// The editing canvas. The composited document lives in a grid of 512 px
/// tile layers so that brush strokes only re-upload the tiles they touch;
/// zooming and panning are pure Core Animation transforms (GPU).
final class CanvasNSView: NSView {
    unowned let model: EditorModel

    // geometry: view is flipped (y down) to match image coordinates
    private(set) var scale: CGFloat = 1
    private(set) var offset: CGPoint = .zero
    private var imageSize: CGSize = .zero
    private var fitted = true

    private let world = CALayer()
    private let imageLayer = CALayer()
    private let checker = CALayer()
    private let antsWhite = CAShapeLayer()
    private let antsBlack = CAShapeLayer()
    private let overlay = CAShapeLayer()
    private let overlayDash = CAShapeLayer()
    private let handles = CAShapeLayer()
    private let brushRing = CAShapeLayer()
    private let compareLayer = CALayer()
    private let compareMask = CALayer()
    private let compareDivider = CALayer()
    private var revealLayer: CALayer?
    private var tiles: [Int: CALayer] = [:]
    private let tileSize = 512

    // interaction state
    enum Drag {
        case none
        case pan(start: CGPoint, offset0: CGPoint)
        case brush
        case marquee(start: CGPoint, ellipse: Bool)
        case lasso([CGPoint])
        case quick([CGPoint])
        case move(start: CGPoint, t0: LayerTransform, began: Bool)
        case scale(start: CGPoint, t0: LayerTransform, began: Bool)
        case rotate(start: CGPoint, t0: LayerTransform, began: Bool)
        case shape(start: CGPoint)
        case crop(start: CGPoint)
        case compare
    }
    private var drag: Drag = .none
    private var dragCurrent: CGPoint = .zero
    private var polygon: [CGPoint] = []
    var cropRect: CGRect? { didSet { updateOverlays() } }
    private var spaceDown = false
    private var mouseInside = false
    private var lastMouse: CGPoint?
    var compareFraction: CGFloat = 0.5

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override var wantsUpdateLayer: Bool { true }

    init(model: EditorModel) {
        self.model = model
        super.init(frame: .zero)
        wantsLayer = true
        layerContentsRedrawPolicy = .never
        setupLayers()
        registerForDraggedTypes([.fileURL, .png, .tiff, NSPasteboard.PasteboardType("public.jpeg"), NSPasteboard.PasteboardType("public.heic")])
        setAccessibilityElement(true)
        setAccessibilityRole(.image)
        setAccessibilityLabel("Image canvas")
    }

    required init?(coder: NSCoder) { fatalError() }

    private var reduceMotion: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }

    private func setupLayers() {
        guard let root = layer else { return }
        root.backgroundColor = NSColor.clear.cgColor
        world.frame = bounds
        root.addSublayer(world)
        checker.zPosition = 0
        world.addSublayer(checker)
        imageLayer.anchorPoint = .zero
        imageLayer.zPosition = 1
        world.addSublayer(imageLayer)
        compareLayer.anchorPoint = .zero
        compareLayer.mask = compareMask
        compareMask.backgroundColor = NSColor.black.cgColor
        compareMask.anchorPoint = .zero
        compareLayer.isHidden = true
        compareLayer.zPosition = 5
        imageLayer.addSublayer(compareLayer)
        for a in [antsWhite, antsBlack] {
            a.fillColor = nil
            a.zPosition = 10
            imageLayer.addSublayer(a)
        }
        antsWhite.strokeColor = NSColor.white.cgColor
        antsBlack.strokeColor = NSColor.black.cgColor
        compareDivider.backgroundColor = NSColor.white.cgColor
        compareDivider.shadowOpacity = 0.5
        compareDivider.shadowRadius = 2
        compareDivider.shadowOffset = .zero
        compareDivider.isHidden = true
        compareDivider.zPosition = 20
        world.addSublayer(compareDivider)
        for o in [overlay, overlayDash] {
            o.fillColor = nil
            o.lineWidth = 1
            o.zPosition = 30
            world.addSublayer(o)
        }
        overlay.strokeColor = NSColor.white.withAlphaComponent(0.95).cgColor
        overlayDash.strokeColor = NSColor.black.withAlphaComponent(0.8).cgColor
        overlayDash.lineDashPattern = [4, 4]
        handles.zPosition = 31
        handles.lineWidth = 1
        world.addSublayer(handles)
        brushRing.fillColor = nil
        brushRing.lineWidth = 1
        brushRing.strokeColor = NSColor.white.cgColor
        brushRing.shadowColor = NSColor.black.cgColor
        brushRing.shadowOpacity = 0.9
        brushRing.shadowRadius = 0.5
        brushRing.shadowOffset = .zero
        brushRing.zPosition = 40
        brushRing.isHidden = true
        world.addSublayer(brushRing)
        updateChecker()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateChecker()
        updateOverlays()
    }

    private func updateChecker() {
        let dark = effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        let a: CGFloat = dark ? 0.30 : 1.0, b: CGFloat = dark ? 0.24 : 0.86
        let cell: CGFloat = 8
        let img = NSImage(size: NSSize(width: cell * 2, height: cell * 2), flipped: false) { r in
            NSColor(white: a, alpha: 1).setFill()
            r.fill()
            NSColor(white: b, alpha: 1).setFill()
            NSRect(x: 0, y: 0, width: cell, height: cell).fill()
            NSRect(x: cell, y: cell, width: cell, height: cell).fill()
            return true
        }
        checker.backgroundColor = NSColor(patternImage: img).cgColor
        checker.shadowColor = NSColor.black.cgColor
        checker.shadowOpacity = dark ? 0.5 : 0.18
        checker.shadowRadius = 8
        checker.shadowOffset = CGSize(width: 0, height: 2)
    }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        world.frame = bounds
        if fitted { applyFit() } else { applyGeometry() }
        CATransaction.commit()
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect, .cursorUpdate], owner: self))
    }

    // MARK: Geometry

    var backing: CGFloat { window?.backingScaleFactor ?? 2 }

    func viewToImage(_ p: CGPoint) -> CGPoint {
        CGPoint(x: (p.x - offset.x) / scale, y: (p.y - offset.y) / scale)
    }

    func imageToView(_ p: CGPoint) -> CGPoint {
        CGPoint(x: p.x * scale + offset.x, y: p.y * scale + offset.y)
    }

    private var imageRectInView: CGRect {
        CGRect(x: offset.x, y: offset.y, width: imageSize.width * scale, height: imageSize.height * scale)
    }

    private func applyGeometry() {
        // world layer isn't flipped: convert y
        // The backing layer of a flipped NSView is geometry-flipped, so the
        // whole layer tree uses image-style y-down coordinates.
        imageLayer.bounds = CGRect(origin: .zero, size: imageSize)
        imageLayer.setAffineTransform(CGAffineTransform(scaleX: scale, y: scale))
        imageLayer.position = offset
        checker.frame = imageRectInView
        let nearest = scale * backing >= 2
        for t in tiles.values {
            t.magnificationFilter = nearest ? .nearest : .linear
            t.minificationFilter = .trilinear
        }
        compareLayer.magnificationFilter = nearest ? .nearest : .linear
        let lw = 1 / scale
        antsWhite.lineWidth = lw
        antsBlack.lineWidth = lw
        antsBlack.lineDashPattern = [NSNumber(value: Double(4 * lw)), NSNumber(value: Double(4 * lw))]
        layoutCompare()
        updateOverlays()
        model.zoom = Double(scale * backing)
    }

    private func clampOffset() {
        // keep at least 64 pt of the image visible
        let r = imageRectInView
        let m: CGFloat = 64
        if r.maxX < m { offset.x += m - r.maxX }
        if r.minX > bounds.width - m { offset.x -= r.minX - (bounds.width - m) }
        if r.maxY < m { offset.y += m - r.maxY }
        if r.minY > bounds.height - m { offset.y -= r.minY - (bounds.height - m) }
    }

    private func fitScale() -> CGFloat {
        guard imageSize.width > 0 else { return 1 }
        let pad: CGFloat = 32
        let s = min((bounds.width - pad * 2) / imageSize.width, (bounds.height - pad * 2) / imageSize.height)
        return max(0.005, min(s, 1)) // never upscale beyond 1 pt per pixel when fitting
    }

    private func applyFit() {
        scale = fitScale()
        offset = CGPoint(x: (bounds.width - imageSize.width * scale) / 2, y: (bounds.height - imageSize.height * scale) / 2)
        applyGeometry()
    }

    private func animate(_ animated: Bool, _ body: () -> Void) {
        CATransaction.begin()
        if animated && !reduceMotion {
            CATransaction.setAnimationDuration(0.22)
            CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(name: .easeOut))
        } else {
            CATransaction.setDisableActions(true)
        }
        body()
        CATransaction.commit()
    }

    /// Re-fit after the canvas rectangle changed because a side panel opened or closed. Layout then follows the
    /// panel's own animation, so no extra animation is added here.
    func fitToViewport() {
        fitted = true
        needsLayout = true
    }

    func zoomToFit(animated: Bool = true) {
        fitted = true
        animate(animated) { applyFit() }
    }

    /// `deviceZoom` = device pixels per image pixel (1 = Actual Size).
    func setZoom(_ deviceZoom: CGFloat, anchor: CGPoint? = nil, animated: Bool = true) {
        guard imageSize.width > 0 else { return }
        let newScale = max(0.005, min(64, deviceZoom / backing))
        let a = anchor ?? CGPoint(x: bounds.midX, y: bounds.midY)
        let ip = viewToImage(a)
        scale = newScale
        offset = CGPoint(x: a.x - ip.x * scale, y: a.y - ip.y * scale)
        fitted = false
        clampOffset()
        animate(animated) { applyGeometry() }
    }

    func zoomBy(_ factor: CGFloat, anchor: CGPoint? = nil, animated: Bool = true) {
        setZoom(scale * backing * factor, anchor: anchor, animated: animated)
    }

    func zoomIn() { zoomBy(steppedFactor(up: true)) }
    func zoomOut() { zoomBy(steppedFactor(up: false)) }

    private func steppedFactor(up: Bool) -> CGFloat {
        let steps: [CGFloat] = [0.05, 0.1, 0.125, 0.167, 0.25, 0.333, 0.5, 0.667, 1, 1.5, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64]
        let z = scale * backing
        if up, let n = steps.first(where: { $0 > z * 1.01 }) { return n / z }
        if !up, let n = steps.last(where: { $0 < z * 0.99 }) { return n / z }
        return 1
    }

    // MARK: Engine → tiles

    func updateFromEngine(full: Bool = false) {
        let frame = model.engine.render()
        let size = CGSize(width: Int(frame.width), height: Int(frame.height))
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if size != imageSize || full {
            let sizeChanged = size != imageSize
            imageSize = size
            tiles.values.forEach { $0.removeFromSuperlayer() }
            tiles = [:]
            if size.width > 0 {
                let cols = (Int(size.width) + tileSize - 1) / tileSize
                let rows = (Int(size.height) + tileSize - 1) / tileSize
                for r in 0..<rows {
                    for c in 0..<cols {
                        let t = CALayer()
                        t.anchorPoint = .zero
                        let x = c * tileSize, y = r * tileSize
                        let w = min(tileSize, Int(size.width) - x), h = min(tileSize, Int(size.height) - y)
                        t.frame = CGRect(x: x, y: y, width: w, height: h)
                        t.contentsGravity = .resize
                        t.isOpaque = false
                        // no edge antialiasing between adjacent tiles (prevents seams)
                        t.edgeAntialiasingMask = []
                        t.allowsEdgeAntialiasing = false
                        imageLayer.insertSublayer(t, at: 0)
                        tiles[r * 10_000 + c] = t
                        updateTile(t, x: x, y: y, w: w, h: h, frame: frame)
                    }
                }
            }
            if sizeChanged, model.tool == .crop { cropRect = size.width > 0 ? CGRect(origin: .zero, size: size) : nil }
            if sizeChanged { if fitted || size.width == 0 { applyFit() } else { clampOffset(); applyGeometry() } }
            else { applyGeometry() }
            // keep image layer's flipped content: tiles use bottom-up frames inside a y-flipped transform
        } else if frame.dirty_w > 0 && frame.dirty_h > 0 {
            let c0 = Int(frame.dirty_x) / tileSize, c1 = (Int(frame.dirty_x) + Int(frame.dirty_w) - 1) / tileSize
            let r0 = Int(frame.dirty_y) / tileSize, r1 = (Int(frame.dirty_y) + Int(frame.dirty_h) - 1) / tileSize
            for r in r0...r1 {
                for c in c0...c1 {
                    guard let t = tiles[r * 10_000 + c] else { continue }
                    let x = c * tileSize, y = r * tileSize
                    let w = min(tileSize, Int(size.width) - x), h = min(tileSize, Int(size.height) - y)
                    updateTile(t, x: x, y: y, w: w, h: h, frame: frame)
                }
            }
        }
        CATransaction.commit()
        setAccessibilityValue(size.width > 0 ? "\(Int(size.width)) by \(Int(size.height)) pixels" : "Empty")
    }

    private func updateTile(_ t: CALayer, x: Int, y: Int, w: Int, h: Int, frame: CutoutFrame) {
        guard let base = frame.data, w > 0, h > 0 else { t.contents = nil; return }
        let stride = Int(frame.width) * 4
        var data = Data(count: w * h * 4)
        data.withUnsafeMutableBytes { dst in
            let d = dst.bindMemory(to: UInt8.self).baseAddress!
            for row in 0..<h {
                memcpy(d + row * w * 4, base + (y + row) * stride + x * 4, w * 4)
            }
        }
        guard let provider = CGDataProvider(data: data as CFData),
              let cg = CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4,
                               space: CGColorSpace(name: CGColorSpace.sRGB)!,
                               bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                               provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent) else { return }
        t.contents = cg
    }

    // MARK: Selection outline (marching ants)

    func updateSelectionOutline() {
        let segs = model.outline
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if segs.isEmpty {
            antsWhite.path = nil
            antsBlack.path = nil
            antsBlack.removeAnimation(forKey: "march")
        } else {
            let p = CGMutablePath()
            var i = 0
            while i + 3 < segs.count {
                p.move(to: CGPoint(x: CGFloat(segs[i]), y: CGFloat(segs[i + 1])))
                p.addLine(to: CGPoint(x: CGFloat(segs[i + 2]), y: CGFloat(segs[i + 3])))
                i += 4
            }
            antsWhite.path = p
            antsBlack.path = p
            if !reduceMotion && antsBlack.animation(forKey: "march") == nil {
                let a = CABasicAnimation(keyPath: "lineDashPhase")
                a.fromValue = 0
                a.toValue = 8 / scale
                a.duration = 0.6
                a.repeatCount = .infinity
                antsBlack.add(a, forKey: "march")
            }
        }
        CATransaction.commit()
    }

    // MARK: Before/after compare and reveal

    func setCompare(_ on: Bool) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if on, let img = model.engine.renderUnmasked(),
           let cg = ImageCodec.cgImage(img, premultiplied: true) {
            compareLayer.contents = cg
            compareLayer.contentsGravity = .resize
            compareLayer.isHidden = false
            compareDivider.isHidden = false
        } else {
            compareLayer.contents = nil
            compareLayer.isHidden = true
            compareDivider.isHidden = true
        }
        layoutCompare()
        CATransaction.commit()
    }

    static func flippedRows(_ img: RGBAImage) -> RGBAImage {
        var out = img
        let row = img.width * 4
        img.data.withUnsafeBytes { src in
            out.data.withUnsafeMutableBytes { dst in
                for y in 0..<img.height {
                    memcpy(dst.baseAddress! + (img.height - 1 - y) * row, src.baseAddress! + y * row, row)
                }
            }
        }
        return out
    }

    private func layoutCompare() {
        // compareLayer lives inside imageLayer (image pixel units)
        compareLayer.frame = CGRect(origin: .zero, size: imageSize)
        compareMask.frame = CGRect(x: 0, y: 0, width: imageSize.width * compareFraction, height: imageSize.height)
        let r = imageRectInView
        let x = r.minX + r.width * compareFraction
        compareDivider.frame = CGRect(x: x - 1, y: r.minY, width: 2, height: r.height)
    }

    /// Called just before the engine applies a background-removal mask: keep a
    /// snapshot of the current pixels and fade it out to reveal transparency.
    func prepareReveal() {
        guard !reduceMotion else { return }
        let snap = CALayer()
        snap.frame = CGRect(origin: .zero, size: imageSize)
        snap.zPosition = 4
        snap.contentsGravity = .resize
        // reuse tile contents
        for t in tiles.values {
            let c = CALayer()
            c.frame = t.frame
            c.contents = t.contents
            c.contentsGravity = .resize
            snap.addSublayer(c)
        }
        imageLayer.addSublayer(snap)
        revealLayer?.removeFromSuperlayer()
        revealLayer = snap
        DispatchQueue.main.async { [weak self] in
            CATransaction.begin()
            CATransaction.setAnimationDuration(0.55)
            CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(name: .easeInEaseOut))
            CATransaction.setCompletionBlock { snap.removeFromSuperlayer() }
            snap.opacity = 0
            CATransaction.commit()
            if self?.revealLayer === snap { self?.revealLayer = nil }
        }
    }

    func announce(_ s: String) {
        NSAccessibility.post(element: self, notification: .announcementRequested, userInfo: [.announcement: s, .priority: NSAccessibilityPriorityLevel.medium.rawValue])
    }

    func toolChanged() {
        polygon = []
        if model.tool != .crop { cropRect = nil }
        if model.tool == .crop, cropRect == nil, imageSize.width > 0 {
            cropRect = CGRect(origin: .zero, size: imageSize)
        }
        updateBrushRing()
        updateOverlays()
        window?.invalidateCursorRects(for: self)
    }

    // MARK: Overlays

    private func viewRectFlipped(_ r: CGRect) -> CGRect { r }

    private func toWorld(_ p: CGPoint) -> CGPoint { p }

    /// Corners of the active layer in view coordinates (tl, tr, br, bl).
    func activeQuad() -> [CGPoint]? {
        guard let l = model.state.activeLayer else { return nil }
        let t = l.transform
        let w = Double(l.width), h = Double(l.height)
        let c = cos(t.rotation), s = sin(t.rotation)
        func map(_ x: Double, _ y: Double) -> CGPoint {
            let lx = (x - w / 2) * t.scaleX, ly = (y - h / 2) * t.scaleY
            return imageToView(CGPoint(x: c * lx - s * ly + t.x, y: s * lx + c * ly + t.y))
        }
        return [map(0, 0), map(w, 0), map(w, h), map(0, h)]
    }

    private func rotateHandlePoint(_ q: [CGPoint]) -> CGPoint {
        let topMid = CGPoint(x: (q[0].x + q[1].x) / 2, y: (q[0].y + q[1].y) / 2)
        let center = CGPoint(x: (q[0].x + q[2].x) / 2, y: (q[0].y + q[2].y) / 2)
        let dx = topMid.x - center.x, dy = topMid.y - center.y
        let len = max(1, hypot(dx, dy))
        return CGPoint(x: topMid.x + dx / len * 26, y: topMid.y + dy / len * 26)
    }

    func updateOverlays() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let path = CGMutablePath()
        let hp = CGMutablePath()
        var dim: CGPath?
        switch drag {
        case .marquee(let s, let ellipse):
            let r = CGRect(x: min(s.x, dragCurrent.x), y: min(s.y, dragCurrent.y), width: abs(dragCurrent.x - s.x), height: abs(dragCurrent.y - s.y))
            let vr = viewRectFlipped(CGRect(origin: imageToView(r.origin), size: CGSize(width: r.width * scale, height: r.height * scale)))
            if ellipse { path.addEllipse(in: vr) } else { path.addRect(vr) }
        case .lasso(let pts), .quick(let pts):
            if case .lasso = drag, let f = pts.first {
                path.move(to: toWorld(imageToView(f)))
                for p in pts.dropFirst() { path.addLine(to: toWorld(imageToView(p))) }
            }
        case .shape(let s):
            let a = toWorld(imageToView(s)), b = toWorld(imageToView(dragCurrent))
            switch ShapeKind(rawValue: model.settings.shapeKind) ?? .rect {
            case .rect: path.addRect(CGRect(x: min(a.x, b.x), y: min(a.y, b.y), width: abs(b.x - a.x), height: abs(b.y - a.y)))
            case .ellipse: path.addEllipse(in: CGRect(x: min(a.x, b.x), y: min(a.y, b.y), width: abs(b.x - a.x), height: abs(b.y - a.y)))
            case .line, .arrow:
                path.move(to: a)
                path.addLine(to: b)
            }
        default: break
        }
        if model.tool == .polygonLasso, let f = polygon.first {
            path.move(to: toWorld(imageToView(f)))
            for p in polygon.dropFirst() { path.addLine(to: toWorld(imageToView(p))) }
            if let m = lastMouse { path.addLine(to: toWorld(m)) }
        }
        if model.tool == .crop, let cr = cropRect, imageSize.width > 0 {
            let vr = viewRectFlipped(CGRect(origin: imageToView(cr.origin), size: CGSize(width: cr.width * scale, height: cr.height * scale)))
            path.addRect(vr)
            // rule of thirds
            for i in 1...2 {
                let fx = vr.minX + vr.width * CGFloat(i) / 3, fy = vr.minY + vr.height * CGFloat(i) / 3
                path.move(to: CGPoint(x: fx, y: vr.minY)); path.addLine(to: CGPoint(x: fx, y: vr.maxY))
                path.move(to: CGPoint(x: vr.minX, y: fy)); path.addLine(to: CGPoint(x: vr.maxX, y: fy))
            }
            let full = CGMutablePath()
            full.addRect(viewRectFlipped(imageRectInView))
            full.addRect(vr)
            dim = full
            for c in [CGPoint(x: vr.minX, y: vr.minY), CGPoint(x: vr.maxX, y: vr.minY), CGPoint(x: vr.maxX, y: vr.maxY), CGPoint(x: vr.minX, y: vr.maxY)] {
                hp.addRect(CGRect(x: c.x - 4, y: c.y - 4, width: 8, height: 8))
            }
        }
        if model.tool == .move, !model.compareOn, let q = activeQuad(), model.state.layers.count > 0 {
            let w = q.map(toWorld)
            path.move(to: w[0])
            for p in w.dropFirst() { path.addLine(to: p) }
            path.closeSubpath()
            for c in w { hp.addEllipse(in: CGRect(x: c.x - 4.5, y: c.y - 4.5, width: 9, height: 9)) }
            let rh = toWorld(rotateHandlePoint(q))
            let topMid = CGPoint(x: (w[0].x + w[1].x) / 2, y: (w[0].y + w[1].y) / 2)
            path.move(to: topMid)
            path.addLine(to: rh)
            hp.addEllipse(in: CGRect(x: rh.x - 5, y: rh.y - 5, width: 10, height: 10))
        }
        overlay.path = path
        overlayDash.path = path
        handles.path = hp
        let accent = NSColor.controlAccentColor
        handles.fillColor = NSColor.white.cgColor
        handles.strokeColor = accent.cgColor
        if let dim {
            overlay.fillColor = NSColor.black.withAlphaComponent(0.45).cgColor
            overlay.fillRule = .evenOdd
            overlay.path = { let p = CGMutablePath(); p.addPath(dim); p.addPath(path); return p }()
        } else {
            overlay.fillColor = nil
        }
        if model.tool == .move {
            overlay.strokeColor = accent.cgColor
            overlayDash.isHidden = true
        } else {
            overlay.strokeColor = NSColor.white.withAlphaComponent(0.95).cgColor
            overlayDash.isHidden = false
        }
        CATransaction.commit()
    }

    private func updateBrushRing() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let t = model.tool
        if t.isBrush, mouseInside, !spaceDown, let m = lastMouse, !model.isEmpty {
            let d = model.settings.size(for: t) * scale
            let r = CGRect(x: m.x - d / 2, y: m.y - d / 2, width: d, height: d)
            brushRing.path = CGPath(ellipseIn: r, transform: nil)
            brushRing.isHidden = false
        } else {
            brushRing.isHidden = true
        }
        CATransaction.commit()
    }

    // MARK: Cursor

    override func resetCursorRects() {
        let c: NSCursor
        if spaceDown { c = .openHand }
        else {
            switch model.tool {
            case .hand: c = .openHand
            case .move: c = .arrow
            case .text: c = .iBeam
            case .eraser, .restore, .brush, .quickSelect: c = .crosshair
            case .zoom: c = .crosshair
            default: c = .crosshair
            }
        }
        addCursorRect(bounds, cursor: c)
    }

    override func cursorUpdate(with event: NSEvent) {
        if spaceDown { NSCursor.openHand.set(); return }
        if model.tool.isBrush { NSCursor.crosshair.set() }
        super.cursorUpdate(with: event)
    }

    // MARK: Scroll / gestures

    override func scrollWheel(with e: NSEvent) {
        guard imageSize.width > 0 else { return }
        let p = convert(e.locationInWindow, from: nil)
        if e.modifierFlags.contains(.command) || (!e.hasPreciseScrollingDeltas && !e.modifierFlags.contains(.shift)) {
            let dy = e.hasPreciseScrollingDeltas ? e.scrollingDeltaY * 0.01 : e.scrollingDeltaY * 0.1
            zoomBy(CGFloat(exp(Double(dy))), anchor: p, animated: false)
            return
        }
        offset.x += e.scrollingDeltaX * (e.hasPreciseScrollingDeltas ? 1 : 8)
        offset.y += e.scrollingDeltaY * (e.hasPreciseScrollingDeltas ? 1 : 8)
        fitted = false
        clampOffset()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        applyGeometry()
        CATransaction.commit()
        updateBrushRing()
    }

    override func magnify(with e: NSEvent) {
        let p = convert(e.locationInWindow, from: nil)
        zoomBy(1 + e.magnification, anchor: p, animated: false)
    }

    override func smartMagnify(with e: NSEvent) {
        let p = convert(e.locationInWindow, from: nil)
        if abs(scale - fitScale()) < 0.001 { setZoom(1, anchor: p) } else { zoomToFit() }
    }

    // MARK: Mouse

    override func mouseEntered(with e: NSEvent) { mouseInside = true; updateBrushRing() }
    override func mouseExited(with e: NSEvent) { mouseInside = false; updateBrushRing() }

    override func mouseMoved(with e: NSEvent) {
        lastMouse = convert(e.locationInWindow, from: nil)
        updateBrushRing()
        if model.tool == .polygonLasso && !polygon.isEmpty { updateOverlays() }
    }

    private func modeFor(_ e: NSEvent) -> String {
        let f = e.modifierFlags
        if f.contains(.shift) && f.contains(.option) { return "intersect" }
        if f.contains(.shift) { return "add" }
        if f.contains(.option) { return "subtract" }
        return model.selectionMode.rawValue
    }

    private func brushArgs(_ p: CGPoint) -> [String: Any] {
        let s = model.settings
        switch model.tool {
        case .eraser:
            return ["mode": "erase", "size": s.eraserSize, "hardness": s.eraserHardness, "opacity": s.eraserStrength, "color": [0, 0, 0, 0], "x": p.x, "y": p.y]
        case .restore:
            return ["mode": "restore", "size": s.restoreSize, "hardness": s.restoreHardness, "opacity": s.restoreStrength, "color": [0, 0, 0, 0], "x": p.x, "y": p.y]
        default:
            return ["mode": "paint", "size": s.brushSize, "hardness": s.brushHardness, "opacity": s.brushOpacity, "color": s.colorRGBA, "x": p.x, "y": p.y]
        }
    }

    override func mouseDown(with e: NSEvent) {
        window?.makeFirstResponder(self)
        let v = convert(e.locationInWindow, from: nil)
        let p = viewToImage(v)
        dragCurrent = p
        guard !model.isEmpty else { return }
        if model.compareOn {
            drag = .compare
            moveCompare(v)
            return
        }
        if spaceDown || model.tool == .hand {
            drag = .pan(start: v, offset0: offset)
            NSCursor.closedHand.set()
            return
        }
        let s = model.settings
        switch model.tool {
        case .zoom:
            zoomBy(e.modifierFlags.contains(.option) ? 0.5 : 2, anchor: v)
        case .move:
            if let q = activeQuad() {
                let rh = rotateHandlePoint(q)
                if hypot(v.x - rh.x, v.y - rh.y) < 10, let t = model.state.activeLayer?.transform {
                    drag = .rotate(start: p, t0: t, began: false); return
                }
                if q.contains(where: { hypot(v.x - $0.x, v.y - $0.y) < 9 }), let t = model.state.activeLayer?.transform {
                    drag = .scale(start: p, t0: t, began: false); return
                }
            }
            // auto-select the top-most layer under the cursor
            if let hit = model.state.layers.reversed().first(where: { $0.visible && $0.bounds.contains(p) }),
               hit.id != model.state.active, !(model.state.activeLayer?.bounds.contains(p) ?? false) {
                model.selectLayer(hit.id)
            }
            if e.clickCount == 2, let l = model.state.activeLayer, l.isText {
                model.sheet = .text(layerID: l.id, point: p)
                return
            }
            if let t = model.state.activeLayer?.transform { drag = .move(start: p, t0: t, began: false) }
        case .rectSelect, .ellipseSelect:
            drag = .marquee(start: p, ellipse: model.tool == .ellipseSelect)
        case .lasso:
            drag = .lasso([p])
        case .polygonLasso:
            if e.clickCount >= 2 || (polygon.count > 2 && hypot(v.x - imageToView(polygon[0]).x, v.y - imageToView(polygon[0]).y) < 8) {
                finishPolygon(mode: modeFor(e))
            } else {
                polygon.append(p)
                updateOverlays()
            }
        case .magicWand:
            model.run("magic_wand", ["x": p.x, "y": p.y, "tolerance": s.tolerance, "contiguous": s.contiguous, "antialias": s.antialias,
                                     "sample_all": s.sampleAll, "combine": modeFor(e), "feather": s.feather])
        case .quickSelect:
            drag = .quick([p])
            let mode = e.modifierFlags.contains(.option) ? "subtract" : "add"
            model.run("quick_select", ["points": [[p.x, p.y]], "radius": s.quickSize / 2, "edge": s.quickEdge, "begin": true, "combine": mode, "sample_all": true])
        case .magicEraser:
            model.run("magic_erase", ["x": p.x, "y": p.y, "tolerance": s.tolerance, "contiguous": s.contiguous, "antialias": s.antialias, "feather": s.eraseFeather])
        case .eraser, .restore, .brush:
            if model.run("brush_begin", brushArgs(p), refresh: false) != nil {
                drag = .brush
                model.canvas?.updateFromEngine()
            } else {
                model.refresh()
            }
        case .fill:
            model.run("fill", ["x": p.x, "y": p.y, "tolerance": s.tolerance, "contiguous": s.contiguous, "color": s.colorRGBA, "opacity": s.fillOpacity, "sample_all": s.sampleAll])
        case .eyedropper:
            if let r = model.run("color_at", ["x": p.x, "y": p.y, "sample_all": true], refresh: false), let c = r["color"] as? [Int] {
                model.settings.color = c.map { Double($0) / 255 }
                model.showToast("Picked colour", symbol: "eyedropper")
            }
        case .text:
            model.sheet = .text(layerID: nil, point: p)
        case .shape:
            drag = .shape(start: p)
        case .crop:
            drag = .crop(start: p)
        case .hand:
            break
        }
    }

    override func mouseDragged(with e: NSEvent) {
        let v = convert(e.locationInWindow, from: nil)
        let p = viewToImage(v)
        dragCurrent = p
        lastMouse = v
        switch drag {
        case .none: break
        case .compare:
            moveCompare(v)
        case .pan(let start, let o0):
            offset = CGPoint(x: o0.x + v.x - start.x, y: o0.y + v.y - start.y)
            fitted = false
            clampOffset()
            CATransaction.begin(); CATransaction.setDisableActions(true); applyGeometry(); CATransaction.commit()
        case .brush:
            if model.run("brush_move", ["x": p.x, "y": p.y], refresh: false, silent: true) != nil {
                updateFromEngine()
            }
            updateBrushRing()
        case .marquee, .shape:
            updateOverlays()
        case .lasso(var pts):
            if let l = pts.last, hypot(l.x - p.x, l.y - p.y) * scale > 2 { pts.append(p); drag = .lasso(pts) }
            updateOverlays()
        case .quick(var pts):
            if let l = pts.last, hypot(l.x - p.x, l.y - p.y) * scale > 3 {
                pts.append(p)
                drag = .quick(pts)
                model.run("quick_select", ["points": [[l.x, l.y], [p.x, p.y]], "radius": model.settings.quickSize / 2, "edge": model.settings.quickEdge, "sample_all": true], silent: true)
            }
            updateBrushRing()
        case .move(let start, let t0, let began):
            if !began { model.run("checkpoint", ["label": "Move Layer"], refresh: false); drag = .move(start: start, t0: t0, began: true) }
            var t = t0
            var dx = p.x - start.x, dy = p.y - start.y
            if e.modifierFlags.contains(.shift) { if abs(dx) > abs(dy) { dy = 0 } else { dx = 0 } }
            t.x += dx; t.y += dy
            setTransform(t)
        case .scale(let start, let t0, let began):
            if !began { model.run("checkpoint", ["label": "Scale Layer"], refresh: false); drag = .scale(start: start, t0: t0, began: true) }
            let c = CGPoint(x: t0.x, y: t0.y)
            let d0 = max(1, hypot(start.x - c.x, start.y - c.y))
            let f = max(0.01, hypot(p.x - c.x, p.y - c.y) / d0)
            var t = t0
            t.scaleX = t0.scaleX * f; t.scaleY = t0.scaleY * f
            setTransform(t)
        case .rotate(let start, let t0, let began):
            if !began { model.run("checkpoint", ["label": "Rotate Layer"], refresh: false); drag = .rotate(start: start, t0: t0, began: true) }
            let c = CGPoint(x: t0.x, y: t0.y)
            var a = t0.rotation + atan2(p.y - c.y, p.x - c.x) - atan2(start.y - c.y, start.x - c.x)
            if e.modifierFlags.contains(.shift) { a = (a / (.pi / 12)).rounded() * (.pi / 12) }
            var t = t0
            t.rotation = a
            setTransform(t)
        case .crop(let s):
            var r = CGRect(x: min(s.x, p.x), y: min(s.y, p.y), width: abs(p.x - s.x), height: abs(p.y - s.y))
            r = r.intersection(CGRect(origin: .zero, size: imageSize))
            cropRect = r.isNull ? nil : r
        }
    }

    private func setTransform(_ t: LayerTransform) {
        guard let id = model.state.active else { return }
        model.run("set_transform", ["id": id, "transform": t.json, "record": false], silent: true)
    }

    override func mouseUp(with e: NSEvent) {
        let p = viewToImage(convert(e.locationInWindow, from: nil))
        let s = model.settings
        switch drag {
        case .brush:
            model.run("brush_end")
        case .pan:
            window?.invalidateCursorRects(for: self)
        case .marquee(let st, let ellipse):
            let r = CGRect(x: min(st.x, p.x), y: min(st.y, p.y), width: abs(p.x - st.x), height: abs(p.y - st.y))
            if r.width * scale < 2 || r.height * scale < 2 {
                if modeFor(e) == "replace" { model.run("deselect") }
            } else {
                model.run(ellipse ? "select_ellipse" : "select_rect", ["x": r.minX, "y": r.minY, "w": r.width, "h": r.height, "combine": modeFor(e), "feather": s.feather])
            }
        case .lasso(let pts):
            if pts.count > 2 {
                model.run("select_polygon", ["points": pts.map { [$0.x, $0.y] }, "combine": modeFor(e), "feather": s.feather])
            }
        case .shape(let st):
            if hypot(p.x - st.x, p.y - st.y) * scale > 3 {
                let kind = ShapeKind(rawValue: s.shapeKind) ?? .rect
                var args: [String: Any] = ["kind": kind.rawValue, "x0": st.x, "y0": st.y, "x1": p.x, "y1": p.y,
                                           "stroke": s.colorRGBA, "width": s.shapeStroke, "corner": s.shapeCorner]
                if s.shapeFilled && (kind == .rect || kind == .ellipse) { args["fill"] = s.fillRGBA }
                model.run("add_shape", args)
            }
        default: break
        }
        drag = .none
        updateOverlays()
    }

    override func rightMouseDown(with e: NSEvent) {
        if let m = menu(for: e) { NSMenu.popUpContextMenu(m, with: e, for: self) }
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        guard !model.isEmpty else {
            let m = NSMenu()
            m.addItem(withTitle: "Paste", action: #selector(menuPaste), keyEquivalent: "").target = self
            m.addItem(withTitle: "Open Image…", action: #selector(menuOpen), keyEquivalent: "").target = self
            return m
        }
        let m = NSMenu()
        func add(_ t: String, _ sel: Selector, enabled: Bool = true) {
            let i = m.addItem(withTitle: t, action: enabled ? sel : nil, keyEquivalent: "")
            i.target = self
        }
        add("Copy", #selector(menuCopy))
        add("Paste as Layer", #selector(menuPaste))
        m.addItem(.separator())
        add("Remove Background", #selector(menuRemoveBG))
        if model.state.hasSelection {
            add("Delete Selected Pixels", #selector(menuDeleteSel))
            add("Keep Only Selection", #selector(menuKeepSel))
            add("Invert Selection", #selector(menuInvert))
            add("Crop to Selection", #selector(menuCropSel))
            add("Deselect", #selector(menuDeselect))
        } else {
            add("Select All", #selector(menuSelectAll))
        }
        m.addItem(.separator())
        add("Duplicate Layer", #selector(menuDuplicate))
        add("Merge Down", #selector(menuMergeDown))
        add("Restore All Erased Pixels", #selector(menuClearMask))
        return m
    }

    @objc private func menuPaste() { model.paste() }
    @objc private func menuOpen() { model.openPanel() }
    @objc private func menuCopy() { model.copy(.copy) }
    @objc private func menuRemoveBG() { model.removeBackground(.best) }
    @objc private func menuDeleteSel() { model.run("delete_selection") }
    @objc private func menuKeepSel() { model.run("mask_from_selection") }
    @objc private func menuInvert() { model.run("invert_selection") }
    @objc private func menuCropSel() { model.run("crop_to_selection") }
    @objc private func menuDeselect() { model.run("deselect") }
    @objc private func menuSelectAll() { model.run("select_all") }
    @objc private func menuDuplicate() { model.run("duplicate_layer") }
    @objc private func menuMergeDown() { model.run("merge_down") }
    @objc private func menuClearMask() { model.run("clear_mask") }

    private func moveCompare(_ v: CGPoint) {
        let r = imageRectInView
        guard r.width > 0 else { return }
        compareFraction = max(0, min(1, (v.x - r.minX) / r.width))
        CATransaction.begin(); CATransaction.setDisableActions(true); layoutCompare(); CATransaction.commit()
    }

    // MARK: Polygon / crop helpers

    func finishPolygon(mode: String? = nil) {
        if polygon.count > 2 {
            model.run("select_polygon", ["points": polygon.map { [$0.x, $0.y] }, "combine": mode ?? model.selectionMode.rawValue, "feather": model.settings.feather])
        }
        polygon = []
        updateOverlays()
    }

    func applyCrop() {
        guard let r = cropRect?.integral, r.width >= 1, r.height >= 1 else { return }
        if r.size == imageSize { return }
        model.run("crop", ["x": r.minX, "y": r.minY, "w": r.width, "h": r.height])
        cropRect = CGRect(origin: .zero, size: imageSize)
        zoomToFit()
    }

    // MARK: Keyboard

    override func keyDown(with e: NSEvent) {
        let flags = e.modifierFlags.intersection([.command, .control, .option])
        if e.keyCode == 49 { // space
            if !spaceDown { spaceDown = true; window?.invalidateCursorRects(for: self); NSCursor.openHand.set(); updateBrushRing() }
            return
        }
        switch e.keyCode {
        case 53: // esc
            if !polygon.isEmpty { polygon = []; updateOverlays(); return }
            if model.tool == .crop { cropRect = CGRect(origin: .zero, size: imageSize); return }
            if model.compareOn { model.compareOn = false; return }
            if model.isRemoving { model.cancelRemoval(); return }
            if model.state.hasSelection { model.run("deselect"); return }
        case 36, 76: // return
            if model.tool == .polygonLasso { finishPolygon(); return }
            if model.tool == .crop { applyCrop(); return }
        case 51, 117: // delete
            if model.state.hasSelection { model.run("delete_selection"); return }
        case 123, 124, 125, 126: // arrows
            if model.tool == .move, flags.isEmpty {
                let step: Double = e.modifierFlags.contains(.shift) ? 10 : 1
                let d: (Double, Double) = [123: (-step, 0), 124: (step, 0), 125: (0, step), 126: (0, -step)][e.keyCode]!
                model.run("nudge", ["dx": d.0, "dy": d.1])
                return
            }
        default: break
        }
        if flags.isEmpty, let ch = e.charactersIgnoringModifiers?.lowercased().first {
            if ch == "[" || ch == "]" {
                let cur = model.settings.size(for: model.tool)
                model.settings.setSize(ch == "]" ? cur * 1.15 + 1 : cur / 1.15 - 1, for: model.tool)
                updateBrushRing()
                return
            }
            if let t = Tool.allCases.first(where: { $0.shortcut == ch }) {
                model.tool = t
                return
            }
        }
        super.keyDown(with: e)
    }

    override func keyUp(with e: NSEvent) {
        if e.keyCode == 49 {
            spaceDown = false
            window?.invalidateCursorRects(for: self)
            updateBrushRing()
            return
        }
        super.keyUp(with: e)
    }

    // MARK: Drag & drop

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        model.dropTargeted = true
        return .copy
    }

    override func draggingExited(_ sender: NSDraggingInfo?) {
        model.dropTargeted = false
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        model.dropTargeted = false
        do {
            let items = try Clipboard.read(from: sender.draggingPasteboard)
            for item in items {
                try model.importImage(item.image, name: item.name, asLayer: !model.isEmpty)
            }
            return true
        } catch {
            // .cutout projects arrive as file URLs that aren't images
            if let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL],
               let p = urls.first(where: { $0.pathExtension.lowercased() == "cutout" }) {
                model.openProject(p)
                return true
            }
            model.showToast(error.localizedDescription, symbol: "exclamationmark.triangle.fill", error: true)
            return false
        }
    }
}
