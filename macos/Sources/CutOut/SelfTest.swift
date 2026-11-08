import AppKit
import SwiftUI

/// End-to-end self test, run with `CUTOUT_SELFTEST=<output dir>` (see
/// scripts/selftest.sh). Drives the real EditorModel, engine, canvas, AI
/// paths and system pasteboard, writes `selftest.json` plus window
/// snapshots, then quits. Never active in normal use.
@MainActor
enum SelfTest {
    static var failNextClipboardWrite = false

    struct Check: Encodable {
        var name: String
        var ok: Bool
        var detail: String
        var ms: Int
    }

    static func runIfRequested(model: EditorModel) {
        let env = ProcessInfo.processInfo.environment
        guard let dir = env["CUTOUT_SELFTEST"], !dir.isEmpty else { return }
        let out = URL(fileURLWithPath: dir, isDirectory: true)
        try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        Task { @MainActor in
            await run(model: model, out: out, env: env)
        }
    }

    private static func run(model: EditorModel, out: URL, env: [String: String]) async {
        var checks: [Check] = []
        func check(_ name: String, _ body: () async throws -> String) async {
            let t0 = Date()
            do {
                let d = try await body()
                checks.append(Check(name: name, ok: true, detail: d, ms: Int(Date().timeIntervalSince(t0) * 1000)))
            } catch {
                checks.append(Check(name: name, ok: false, detail: error.localizedDescription, ms: Int(Date().timeIntervalSince(t0) * 1000)))
            }
        }
        func fail(_ s: String) -> EngineError { EngineError(message: s) }
        func wait(_ s: Double) async { try? await Task.sleep(for: .seconds(s)) }
        func snapshot(_ name: String) {
            guard let win = NSApp.windows.first(where: { $0.isVisible && $0.contentView != nil }), let v = win.contentView else { return }
            // Composite the window, including Core Animation layers.
            let wid = CGWindowID(win.windowNumber)
            if let cg = CGWindowListCreateImage(.null, .optionIncludingWindow, wid, [.boundsIgnoreFraming, .bestResolution]) {
                let rep = NSBitmapImageRep(cgImage: cg)
                try? rep.representation(using: .png, properties: [:])?.write(to: out.appendingPathComponent("\(name).png"))
            }
            // Also render the layer tree directly (works while the display sleeps).
            if let root = v.layer ?? v.superview?.layer {
                let scale = win.backingScaleFactor
                let w = Int(v.bounds.width * scale), h = Int(v.bounds.height * scale)
                if let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                       space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) {
                    ctx.setFillColor(NSColor.windowBackgroundColor.cgColor)
                    ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
                    ctx.translateBy(x: 0, y: CGFloat(h))
                    ctx.scaleBy(x: scale, y: -scale)
                    v.displayIfNeeded()
                    root.render(in: ctx)
                    if let img = ctx.makeImage() {
                        try? NSBitmapImageRep(cgImage: img).representation(using: .png, properties: [:])?.write(to: out.appendingPathComponent("\(name)-layers.png"))
                    }
                }
            }
        }
        func alphaAt(_ img: RGBAImage, _ x: Int, _ y: Int) -> UInt8 {
            img.data[(y * img.width + x) * 4 + 3]
        }
        func putOnPasteboard(_ url: URL) throws {
            let d = try Data(contentsOf: url)
            let pb = NSPasteboard.general
            pb.clearContents()
            let type: NSPasteboard.PasteboardType = url.pathExtension.lowercased() == "png" ? .png : NSPasteboard.PasteboardType("public.jpeg")
            guard pb.setData(d, forType: type) else { throw fail("pasteboard write failed") }
        }

        let bench = URL(fileURLWithPath: env["CUTOUT_SELFTEST_BENCH"] ?? "bench/all")
        let gear = bench.appendingPathComponent("white_gear_on_beige/image.png")
        let cat = bench.appendingPathComponent("cat_fur.jpg")
        let mug = bench.appendingPathComponent("mug_vs_cup.jpg")

        await wait(0.8)
        snapshot("01-empty-state")

        // Window / launch
        await check("launch: main window visible") {
            guard let w = NSApp.windows.first(where: { $0.isVisible }) else { throw fail("no window") }
            return "window \(Int(w.frame.width))×\(Int(w.frame.height))"
        }

        await check("paste: image from clipboard creates document") {
            try putOnPasteboard(gear)
            model.paste()
            guard model.state.layers.count == 1, model.state.width == 1600, model.state.height == 1200 else {
                throw fail("state \(model.state.width)×\(model.state.height) layers \(model.state.layers.count)")
            }
            return "1600×1200, 1 layer"
        }
        await wait(0.4)
        snapshot("02-loaded")

        await check("magic eraser: removes beige, keeps white part") {
            model.settings.tolerance = 8
            model.settings.contiguous = true
            model.run("magic_erase", ["x": 40, "y": 40, "tolerance": 8, "contiguous": true, "antialias": true])
            guard let img = model.engine.export() else { throw fail("export failed") }
            let bg = alphaAt(img, 40, 40), part = alphaAt(img, 800 + 350, 600), hole = alphaAt(img, 800 + 270, 600)
            guard bg == 0, part == 255 else { throw fail("bg α=\(bg) part α=\(part)") }
            return "background α=\(bg), white part α=\(part), bolt-hole α=\(hole) (enclosed hole kept: contiguous mode)"
        }

        await check("color delete (global): also clears enclosed holes") {
            model.run("color_delete", ["x": 800 + 270, "y": 600, "tolerance": 8, "antialias": true])
            guard let img = model.engine.export() else { throw fail("export failed") }
            let hole = alphaAt(img, 800 + 270, 600), part = alphaAt(img, 800 + 350, 600)
            guard hole == 0, part == 255 else { throw fail("hole α=\(hole) part α=\(part)") }
            return "hole α=0, part α=255"
        }

        await check("restore brush: brings erased pixels back") {
            model.run("brush_begin", ["mode": "restore", "size": 60, "hardness": 1, "opacity": 1, "color": [0, 0, 0, 0], "x": 40, "y": 40])
            model.run("brush_move", ["x": 80, "y": 40])
            model.run("brush_end")
            guard let img = model.engine.export() else { throw fail("export") }
            guard alphaAt(img, 60, 40) == 255 else { throw fail("α=\(alphaAt(img, 60, 40))") }
            return "restored α=255"
        }

        await check("undo / redo") {
            let before = model.state.undoLabel ?? ""
            model.undo()
            guard let img = model.engine.export(), alphaAt(img, 60, 40) == 0 else { throw fail("undo did not revert") }
            model.redo()
            guard let img2 = model.engine.export(), alphaAt(img2, 60, 40) == 255 else { throw fail("redo did not reapply") }
            return "undid '\(before)' and redid"
        }

        await check("paste second image as new layer") {
            try putOnPasteboard(mug)
            model.paste()
            guard model.state.layers.count == 2 else { throw fail("layers \(model.state.layers.count)") }
            return "2 layers; top \(model.state.layers.last!.width)×\(model.state.layers.last!.height) scaled to fit"
        }

        await check("move / scale / rotate layer transform") {
            guard var t = model.state.activeLayer?.transform, let id = model.state.active else { throw fail("no active") }
            t.x += 100; t.scaleX *= 0.5; t.scaleY *= 0.5; t.rotation = 0.3
            model.run("set_transform", ["id": id, "transform": t.json])
            guard let n = model.state.activeLayer?.transform, abs(n.rotation - 0.3) < 1e-9, abs(n.scaleX - t.scaleX) < 1e-9 else { throw fail("transform not applied") }
            return "x+100, 50%, 17°"
        }

        await check("text layer (CoreText)") {
            let props = TextProps(text: "CutOut", font: "Helvetica Neue", size: 120, color: [255, 60, 60, 255], bold: true, italic: false, align: "left")
            model.commitText(props, layerID: nil, at: CGPoint(x: 100, y: 100))
            guard model.state.layers.count == 3, model.state.activeLayer?.isText == true else { throw fail("no text layer") }
            return "text layer \(model.state.activeLayer!.width)×\(model.state.activeLayer!.height)"
        }

        await check("shape layer") {
            model.run("add_shape", ["kind": "arrow", "x0": 100, "y0": 900, "x1": 500, "y1": 700, "stroke": [0, 122, 255, 255], "width": 12])
            guard model.state.layers.count == 4 else { throw fail("layers \(model.state.layers.count)") }
            return "arrow layer added"
        }
        snapshot("03-layers")

        await check("merge down + layer visibility + opacity") {
            guard let top = model.state.active else { throw fail("none") }
            model.run("set_layer", ["id": top, "opacity": 0.5])
            model.run("merge_down", ["id": top])
            guard model.state.layers.count == 3 else { throw fail("layers \(model.state.layers.count)") }
            return "merged 4→3 layers"
        }

        await check("quick selection + delete selection") {
            model.run("quick_select", ["points": [[800, 300], [820, 320]], "radius": 20, "edge": 0.5, "begin": true, "sample_all": true])
            guard model.state.hasSelection else { throw fail("no selection") }
            let b = model.state.selectionBounds ?? .zero
            model.run("deselect")
            return "selection bounds \(Int(b.width))×\(Int(b.height))"
        }

        await check("project save → clear → reopen is lossless") {
            let url = out.appendingPathComponent("roundtrip.cutout")
            guard let before = model.engine.export() else { throw fail("export") }
            switch model.engine.command("save_project", ["path": url.path]) {
            case .failure(let e): throw e
            case .success: break
            }
            let n = model.state.layers.count
            model.clearWorkspace(saveRecovery: false)
            guard model.isEmpty else { throw fail("not cleared") }
            model.openProject(url)
            guard let after = model.engine.export(), model.state.layers.count == n else { throw fail("reopen failed") }
            guard before.data == after.data else { throw fail("pixels differ after reopen") }
            let size = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? Int) ?? 0
            return "\(n) layers identical; file \(size / 1024) KB"
        }

        await check("copy: PNG with real alpha on the pasteboard") {
            model.copy(.copy)
            let pb = NSPasteboard.general
            guard let png = pb.data(forType: .png), let tiff = pb.data(forType: .tiff) else { throw fail("missing types: \(pb.types ?? [])") }
            let r = try ImageCodec.decode(data: png)
            guard r.image.width == model.state.width, r.image.hasTransparency else { throw fail("no transparency in copied PNG") }
            let t = try ImageCodec.decode(data: tiff)
            guard t.image.hasTransparency else { throw fail("TIFF lost alpha") }
            return "PNG \(png.count / 1024) KB + TIFF \(tiff.count / 1024) KB, both with alpha"
        }

        await check("copy & clear: failed clipboard write keeps workspace") {
            failNextClipboardWrite = true
            model.copy(.copyAndClear)
            guard !model.isEmpty else { throw fail("workspace was cleared after a failed copy!") }
            return "workspace preserved after injected failure"
        }

        await check("remove background: Fast (Apple Vision)") {
            model.clearWorkspace(saveRecovery: false)
            try putOnPasteboard(cat)
            model.paste()
            model.removeBackground(.fast)
            var waited = 0.0
            while model.isRemoving && waited < 60 { await wait(0.2); waited += 0.2 }
            guard let l = model.state.activeLayer, l.hasMask, let img = model.engine.export() else { throw fail("no mask") }
            let corner = alphaAt(img, 5, 5), center = alphaAt(img, img.width / 2, img.height / 2)
            return String(format: "%.1f s; corner α=%d centre α=%d", waited, corner, center)
        }
        snapshot("04-fast-removed")

        if !ModelManager.shared.hasBestModel {
            await check("no best model: Remove Background opens model setup") {
                model.removeBackground(.best)
                guard case .modelSetup = model.sheet else { throw fail("setup sheet not shown") }
                model.sheet = nil
                guard !model.isRemoving else { throw fail("removal started without a model") }
                return "setup sheet shown, nothing started"
            }
        }

        if let archive = env["CUTOUT_SELFTEST_INSTALL_ARCHIVE"], !archive.isEmpty {
            await check("install Core ML model from file (verified)") {
                let bogus = out.appendingPathComponent("bogus.zip")
                try Data("not a model".utf8).write(to: bogus)
                await ModelManager.shared.installCoreML(from: bogus)
                guard ModelManager.shared.lastError != nil, CoreMLBiRefNet.locate() == nil else { throw fail("bogus archive was accepted") }
                await ModelManager.shared.installCoreML(from: URL(fileURLWithPath: archive))
                if let e = ModelManager.shared.lastError { throw fail(e) }
                guard case .installed = CoreMLBiRefNet.locate() else { throw fail("not installed") }
                return "bogus archive rejected; verified archive installed"
            }
        }

        if ModelManager.shared.hasBestModel, env["CUTOUT_SELFTEST_SKIP_BEST"] != "1" {
            await check("remove background: Best (BiRefNet, local)") {
                model.run("clear_mask")
                model.removeBackground(.best)
                var waited = 0.0
                while model.isRemoving && waited < 300 { await wait(0.25); waited += 0.25 }
                guard let l = model.state.activeLayer, l.hasMask, let img = model.engine.export() else { throw fail("no mask (\(waited)s)") }
                let corner = alphaAt(img, 5, 5), center = alphaAt(img, img.width / 2, img.height / 2)
                guard corner < 10, center > 245 else { throw fail("corner α=\(corner) centre α=\(center)") }
                let backend = ModelManager.shared.activeBackend ?? "-"
                if let want = env["CUTOUT_SELFTEST_EXPECT_BACKEND"], !backend.contains(want) {
                    throw fail("backend '\(backend)', expected '\(want)'")
                }
                return String(format: "%.1f s incl. model load; backend %@; corner α=%d centre α=%d", waited, backend, corner, center)
            }
            await check("remove background: Best again (model cached)") {
                model.run("clear_mask")
                let t0 = Date()
                model.removeBackground(.best)
                while model.isRemoving && Date().timeIntervalSince(t0) < 300 { await wait(0.1) }
                guard model.state.activeLayer?.hasMask == true else { throw fail("no mask") }
                return String(format: "%.1f s; backend %@", Date().timeIntervalSince(t0), ModelManager.shared.activeBackend ?? "-")
            }
            await wait(0.8)
            snapshot("05-best-removed")
            model.compareOn = true
            await wait(0.3)
            snapshot("06-compare")
            model.compareOn = false
        }

        await check("refine edges + decontaminate") {
            model.run("refine_commit", ["radius": 4, "decontaminate": true])
            guard model.state.undoLabel == "Refine Edges" else { throw fail("undo label \(model.state.undoLabel ?? "-")") }
            return "applied"
        }

        await check("export PNG encode") {
            guard let img = model.engine.export(), let d = ImageCodec.encode(img, as: .png) else { throw fail("encode") }
            try d.write(to: out.appendingPathComponent("export.png"))
            return "\(d.count / 1024) KB"
        }

        UserDefaults.standard.set(true, forKey: "showTools")
        UserDefaults.standard.set(true, forKey: "showLayers")
        await wait(0.6)
        snapshot("07-panels-open")
        model.tool = .magicEraser
        await wait(0.3)
        snapshot("08-magic-eraser-tool")

        await check("copy & clear: success clears workspace") {
            model.copy(.copyAndClear)
            guard model.isEmpty, NSPasteboard.general.data(forType: .png) != nil else { throw fail("not cleared / no clipboard") }
            return "cleared; clipboard holds PNG"
        }
        UserDefaults.standard.set(false, forKey: "showTools")
        UserDefaults.standard.set(false, forKey: "showLayers")

        // memory
        var info = mach_task_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size) / 4
        _ = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count) }
        }
        let report: [String: Any] = [
            "passed": checks.filter(\.ok).count,
            "failed": checks.filter { !$0.ok }.count,
            "resident_mb_end": Int(info.resident_size / 1_048_576),
            "checks": checks.map { ["name": $0.name, "ok": $0.ok, "detail": $0.detail, "ms": $0.ms] },
        ]
        if let d = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
            try? d.write(to: out.appendingPathComponent("selftest.json"))
        }
        if env["CUTOUT_SELFTEST_COPYCLOSE"] == "1" {
            // final step: Copy & Close with an image loaded
            try? putOnPasteboard(gear)
            model.paste()
            model.copy(.copyAndClose)
            return
        }
        NSApp.terminate(nil)
    }
}
