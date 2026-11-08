import AppKit
import SwiftUI

/// In-process click-alignment test, run with `CUTOUT_HITTEST=<json file>`
/// (+ `CUTOUT_HITTEST_IMAGE=<image>`). It reads each tool button's accessibility
/// frame (where the control is rendered), then posts real mouse events to the
/// window at the centre and near the edges of that frame and records which tool
/// became active. Needs no Accessibility permission since it stays in-process.
@MainActor
enum HitTest {
    struct Row: Encodable { var tool: String; var point: String; var hit: String; var ok: Bool }

    static func runIfRequested(model: EditorModel) {
        let env = ProcessInfo.processInfo.environment
        guard let out = env["CUTOUT_HITTEST"], !out.isEmpty else { return }
        Task { @MainActor in await run(model: model, out: URL(fileURLWithPath: out), image: env["CUTOUT_HITTEST_IMAGE"]) }
    }

    /// Rendered frames (window coordinates, top-left origin) reported by the live ToolButton views.
    static var frames: [Tool: CGRect] = [:]
    static let enabled = ProcessInfo.processInfo.environment["CUTOUT_HITTEST"] != nil || ProcessInfo.processInfo.environment["CUTOUT_CLICKLOG"] != nil
    private static var monitor: Any?

    /// `CUTOUT_CLICKLOG=<file>`: appends one line per real left-click — pointer position, which tool button is drawn
    /// there (from the live SwiftUI frames) and which tool is active 0.3 s later. For diagnosing real-mouse input.
    static func startClickLogIfRequested(model: EditorModel) {
        guard let path = ProcessInfo.processInfo.environment["CUTOUT_CLICKLOG"], !path.isEmpty, monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown]) { ev in
            guard let w = ev.window, let root = w.contentView else { return ev }
            let p = ev.locationInWindow
            let top = CGPoint(x: p.x, y: root.bounds.height - p.y)          // window coords, y down
            let drawn = frames.first(where: { $0.value.contains(top) })?.key.rawValue ?? "-"
            let near = frames.min(by: { hypot($0.value.midX - top.x, $0.value.midY - top.y) < hypot($1.value.midX - top.x, $1.value.midY - top.y) })
            let before = model.tool.rawValue
            let hitView = root.hitTest(root.convert(p, from: nil)).map { "\(type(of: $0))" } ?? "-"
            var line = "click window(top-left)=(\(Int(top.x)),\(Int(top.y))) screen=\(NSEvent.mouseLocation) scale=\(w.backingScaleFactor) drawnToolAtPoint=\(drawn) nearest=\(near?.key.rawValue ?? "-") nearestCentre=(\(Int(near?.value.midX ?? 0)),\(Int(near?.value.midY ?? 0))) toolBefore=\(before) key=\(w.isKeyWindow) hitView=\(hitView.prefix(40))"
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                line += " toolAfter=\(model.tool.rawValue)\n"
                if let h = FileHandle(forWritingAtPath: path) ?? { FileManager.default.createFile(atPath: path, contents: nil); return FileHandle(forWritingAtPath: path) }() {
                    h.seekToEndOfFile(); h.write(Data(line.utf8)); try? h.close()
                }
            }
            return ev
        }
    }

    private static func click(_ w: NSWindow, at p: NSPoint) {
        for (t, n) in [(NSEvent.EventType.mouseMoved, 0), (.leftMouseDown, 1), (.leftMouseUp, 1)] {
            if let ev = NSEvent.mouseEvent(with: t, location: p, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                           windowNumber: w.windowNumber, context: nil, eventNumber: 0, clickCount: n, pressure: 1) {
                w.sendEvent(ev)
            }
        }
    }

    private static func pump(_ s: Double) async { try? await Task.sleep(nanoseconds: UInt64(s * 1e9)) }

    private static func run(model: EditorModel, out: URL, image: String?) async {
        var rows: [Row] = []
        if let image { model.open(urls: [URL(fileURLWithPath: image)], asLayers: false) }
        UserDefaults.standard.set(true, forKey: "showTools")
        await pump(2.5)
        guard let w = NSApp.windows.first(where: { $0.isVisible && $0.contentView != nil }), let root = w.contentView else {
            try? "no window".write(to: out, atomically: true, encoding: .utf8); NSApp.terminate(nil); return
        }
        let found = frames
        if let rep = root.bitmapImageRepForCachingDisplay(in: root.bounds) {
            root.cacheDisplay(in: root.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: out.appendingPathExtension("png"))
        }
        let H = root.bounds.height   // .global == the window content view, y down
        for (tool, g) in found {
            let wr = CGRect(x: g.minX, y: H - g.maxY, width: g.width, height: g.height)
            let pts: [(String, NSPoint)] = [
                ("center", NSPoint(x: wr.midX, y: wr.midY)),
                ("left", NSPoint(x: wr.minX + 3, y: wr.midY)), ("right", NSPoint(x: wr.maxX - 3, y: wr.midY)),
                ("top", NSPoint(x: wr.midX, y: wr.maxY - 3)), ("bottom", NSPoint(x: wr.midX, y: wr.minY + 3)),
            ]
            for (name, p) in pts {
                model.tool = tool == .hand ? .move : .hand     // start from a different tool
                await pump(0.05)
                click(w, at: p)
                await pump(0.15)
                rows.append(Row(tool: tool.rawValue, point: "\(name) @\(Int(p.x)),\(Int(p.y)) frame \(Int(wr.minX)),\(Int(wr.minY)) \(Int(wr.width))x\(Int(wr.height))", hit: model.tool.rawValue, ok: model.tool == tool))
            }
        }
        if let g = found[.move] {
            var lines: [String] = []
            for dy in [-12, 8] {
                let p = NSPoint(x: g.midX, y: H - g.midY + CGFloat(dy))
                var v = root.superview?.hitTest(w.contentView!.convert(p, to: root.superview)) ?? root.hitTest(p)
                var chain: [String] = []
                while let x = v { chain.append("\(type(of: x))"); v = x.superview }
                lines.append("dy \(dy) @\(p): " + chain.joined(separator: " < "))
            }
            lines.append("contentLayoutRect \(w.contentLayoutRect) frame \(w.frame) root \(root.frame) H \(H) styleMask \(w.styleMask.rawValue)")
            try? lines.joined(separator: "\n").write(to: out.appendingPathExtension("views"), atomically: true, encoding: .utf8)
        }
        do {
            var lines: [String] = []
            func walk(_ v: NSView, _ d: Int) {
                let n = "\(type(of: v))"
                if let sv = v as? NSScrollView { lines.append("SCROLL \(n) insets=\(sv.contentInsets) auto=\(sv.automaticallyAdjustsContentInsets) clipBounds=\(sv.contentView.bounds) doc=\(sv.documentView?.frame ?? .zero) docType=\(sv.documentView.map { "\(type(of: $0))" } ?? "-")") }
                if d < 9 || n.contains("Hosting") || n.contains("Scroll") || n.contains("Clip") { lines.append("\(String(repeating: " ", count: d))\(n.prefix(70)) frame=\(v.frame) flipped=\(v.isFlipped) safe=\(v.safeAreaInsets) bounds=\(v.bounds.origin)") }
                if d < 12 { v.subviews.forEach { walk($0, d + 1) } }
            }
            walk(w.contentView!.superview ?? w.contentView!, 0)
            try? lines.prefix(80).joined(separator: "\n").write(to: out.appendingPathExtension("tree"), atomically: true, encoding: .utf8)
        }
        // Layers inspector: eye toggle (drawn at window y≈143) and Duplicate button (drawn at y≈764), scanned vertically.
        if ProcessInfo.processInfo.environment["CUTOUT_HITTEST_LAYERS"] != nil {
            UserDefaults.standard.set(true, forKey: "showLayers"); await pump(1.0)
            var lines: [String] = []
            let winH = root.bounds.height
            var eyeHits: [Int] = [], dupHits: [Int] = []
            for yTop in stride(from: 100, through: 220, by: 6) {
                let before = model.state.layers.first?.visible ?? true
                click(w, at: NSPoint(x: 945, y: winH - CGFloat(yTop))); await pump(0.2)
                if (model.state.layers.first?.visible ?? true) != before { eyeHits.append(yTop); model.run("set_layer", ["id": model.state.layers[0].id, "visible": before]); await pump(0.2) }
            }
            for yTop in stride(from: 690, through: 779, by: 3) {
                let before = model.state.layers.count
                click(w, at: NSPoint(x: 967, y: winH - CGFloat(yTop))); await pump(0.25)
                if model.state.layers.count != before { dupHits.append(yTop); model.run("undo"); await pump(0.25) }
            }
            lines.append("eye toggle responds at window y(top)= \(eyeHits)  (drawn ≈125–160)")
            lines.append("duplicate button responds at window y(top)= \(dupHits)  (drawn ≈ 745–785)")
            try? lines.joined(separator: "\n").write(to: out.appendingPathExtension("layers"), atomically: true, encoding: .utf8)
        }
        // Click test (centre of every tool) repeated after state changes: toggles, resize, tool changes.
        if ProcessInfo.processInfo.environment["CUTOUT_HITTEST_STATES"] != nil {
            var lines: [String] = []
            let ud = UserDefaults.standard
            func centres(_ label: String) async {
                var bad: [String] = []
                for (tool, g) in frames.sorted(by: { $0.key.rawValue < $1.key.rawValue }) {
                    model.tool = tool == .hand ? .move : .hand; await pump(0.05)
                    click(w, at: NSPoint(x: g.midX, y: root.bounds.height - g.midY)); await pump(0.12)
                    if model.tool != tool { bad.append(tool.rawValue) }
                }
                lines.append("\(label): \(frames.count - bad.count)/\(frames.count) ok \(bad.isEmpty ? "" : "FAIL " + bad.joined(separator: ","))")
            }
            await centres("initial")
            ud.set(false, forKey: "showTools"); await pump(0.8); ud.set(true, forKey: "showTools"); await pump(0.8)
            await centres("after tools off/on")
            ud.set(true, forKey: "showLayers"); await pump(0.8)
            await centres("layers open")
            ud.set(false, forKey: "showLayers"); await pump(0.8)
            await centres("layers closed")
            var f = w.frame
            f.size.width = 900; f.size.height = 620; w.setFrame(f, display: true, animate: false); await pump(0.8)
            await centres("window resized 900x620")
            f.size.width = 1500; f.size.height = 900; w.setFrame(f, display: true, animate: false); await pump(0.8)
            await centres("window resized 1500x900")
            for t in [Tool.brush, .text, .crop, .magicWand] { model.tool = t; await pump(0.4) }
            await centres("after selecting other tools (taller options)")
            try? lines.joined(separator: "\n").write(to: out.appendingPathExtension("states"), atomically: true, encoding: .utf8)
        }
        // Tool-option controls (Magic Eraser): which control does a click at each point change?
        if ProcessInfo.processInfo.environment["CUTOUT_HITTEST_OPTIONS"] != nil {
            model.tool = .magicEraser; await pump(0.6)
            var cells: [String] = []
            let winH = root.bounds.height
            for yTop in stride(from: 470, through: 640, by: 6) {
                var row = "\(yTop) "
                for x in stride(from: 16, through: 250, by: 8) {
                    let s0 = model.settings
                    click(w, at: NSPoint(x: CGFloat(x), y: winH - CGFloat(yTop))); await pump(0.06)
                    let s1 = model.settings
                    var c = "."
                    if s1.tolerance != s0.tolerance { c = "T" } else if s1.contiguous != s0.contiguous { c = "C" }
                    else if s1.antialias != s0.antialias { c = "A" } else if s1.eraseFeather != s0.eraseFeather || s1.feather != s0.feather { c = "F" }
                    row += c
                    model.settings = s0; await pump(0.02)
                }
                cells.append(row)
            }
            try? cells.joined(separator: "\n").write(to: out.appendingPathExtension("options"), atomically: true, encoding: .utf8)
        }
        // Centering: toggle both panels repeatedly (also after a manual zoom) and check the image is fitted and centred.
        do {
            var lines: [String] = []
            func report(_ label: String) {
                guard let c = model.canvas else { return }
                let o = c.imageToView(.zero), e = c.imageToView(CGPoint(x: model.state.width, y: model.state.height))
                let cx = (o.x + e.x) / 2 - c.bounds.midX, cy = (o.y + e.y) / 2 - c.bounds.midY
                let fitW = (e.x - o.x), fitH = (e.y - o.y)
                let inside = o.x >= 0 && o.y >= 0 && e.x <= c.bounds.width && e.y <= c.bounds.height
                lines.append("\(label): canvas \(Int(c.bounds.width))x\(Int(c.bounds.height)) image \(Int(fitW))x\(Int(fitH)) centre error (\(String(format: "%.2f", cx)), \(String(format: "%.2f", cy))) fullyInside=\(inside) zoom=\(String(format: "%.3f", c.scale))")
            }
            let ud = UserDefaults.standard
            report("start")
            for round in 1...3 {
                model.canvas?.setZoom(1.0, animated: false); await pump(0.2)
                report("r\(round) manual zoom 100%")
                ud.set(false, forKey: "showTools"); await pump(0.8); report("r\(round) tools closed")
                ud.set(true, forKey: "showLayers"); await pump(0.8); report("r\(round) layers open, tools closed")
                ud.set(true, forKey: "showTools"); await pump(0.8); report("r\(round) both open")
                ud.set(false, forKey: "showLayers"); await pump(0.8); report("r\(round) tools open, layers closed")
            }
            try? lines.joined(separator: "\n").write(to: out.appendingPathExtension("center"), atomically: true, encoding: .utf8)
        }
        // Fidelity probe: the toolbar's Layers button must respond where it is drawn (window y ≈ 27 from top).
        do {
            var probe: [String] = []
            for yTop in [-25, 0, 27, 54, 79] {
                let before = UserDefaults.standard.bool(forKey: "showLayers")
                click(w, at: NSPoint(x: 1089, y: root.bounds.height - CGFloat(yTop)))
                await pump(0.4)
                let after = UserDefaults.standard.bool(forKey: "showLayers")
                probe.append("yTop \(yTop): toggled=\(before != after)")
                if before != after { UserDefaults.standard.set(before, forKey: "showLayers"); await pump(0.4) }
            }
            try? probe.joined(separator: "\n").write(to: out.appendingPathExtension("probe"), atomically: true, encoding: .utf8)
        }
        // Dense grid: which tool does a click at each window point activate? (window coords, y from top)
        if ProcessInfo.processInfo.environment["CUTOUT_HITTEST_GRID"] != nil {
            var cells: [String] = []
            let winH = root.bounds.height
            for yTop in stride(from: 100, through: 560, by: 8) {
                for x in stride(from: 4, through: 252, by: 8) {
                    var res = "-"
                    for neutral in [Tool.hand, Tool.move] {
                        model.tool = neutral
                        click(w, at: NSPoint(x: CGFloat(x), y: winH - CGFloat(yTop)))
                        await pump(0.02)
                        if model.tool != neutral { res = model.tool.rawValue; break }
                    }
                    cells.append("\(x),\(yTop),\(res)")
                }
            }
            try? cells.joined(separator: "\n").write(to: out.appendingPathExtension("grid"), atomically: true, encoding: .utf8)
        }
        // Vertical scan: which y offsets (relative to the rendered frame's centre) activate each tool?
        var scan: [String] = []
        for (tool, g) in found.sorted(by: { $0.key.rawValue < $1.key.rawValue }) where ProcessInfo.processInfo.environment["CUTOUT_HITTEST_SCAN"] != nil {
            let cx = g.midX, cy = H - g.midY
            var works: [Int] = []
            for dy in stride(from: -140, through: 140, by: 4) {
                model.tool = tool == .hand ? .move : .hand
                await pump(0.03)
                click(w, at: NSPoint(x: cx, y: cy + CGFloat(dy)))
                await pump(0.08)
                if model.tool == tool { works.append(dy) }
            }
            scan.append("\(tool.rawValue) frame y(top-left)=\(Int(g.minY))..\(Int(g.maxY)) works dy(y-up)=\(works)")
        }
        try? scan.joined(separator: "\n").write(to: out.appendingPathExtension("scan"), atomically: true, encoding: .utf8)
        let enc = JSONEncoder(); enc.outputFormatting = .prettyPrinted
        try? enc.encode(["buttons": "\(found.count)", "tested": "\(rows.count)", "failed": "\(rows.filter { !$0.ok }.count)"]).write(to: out.appendingPathExtension("summary"))
        try? enc.encode(rows).write(to: out)
        NSApp.terminate(nil)
    }
}
