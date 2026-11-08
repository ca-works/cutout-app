import CCutOut
import CoreGraphics
import Foundation

/// Errors surfaced from the Rust engine.
struct EngineError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// Thin Swift wrapper over the C API. Must be used from the main thread.
@MainActor
final class EngineBridge {
    private let handle: OpaquePointer

    init() {
        guard let h = cutout_engine_new() else { fatalError("Could not create the CutOut engine") }
        handle = h
    }

    deinit {
        cutout_engine_free(handle)
    }

    // MARK: Commands

    @discardableResult
    func command(_ cmd: String, _ args: [String: Any] = [:]) -> Result<[String: Any], EngineError> {
        var payload = args
        payload["cmd"] = cmd
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8) else {
            return .failure(EngineError(message: "Invalid command"))
        }
        guard let reply = cutout_command(handle, json) else {
            return .failure(EngineError(message: "No reply from engine"))
        }
        defer { cutout_string_free(reply) }
        let str = String(cString: reply)
        guard let obj = (try? JSONSerialization.jsonObject(with: Data(str.utf8))) as? [String: Any] else {
            return .failure(EngineError(message: "Malformed reply"))
        }
        if obj["ok"] as? Bool == true { return .success(obj) }
        return .failure(EngineError(message: obj["error"] as? String ?? "Unknown error"))
    }

    func state() -> EngineState {
        guard case .success(let s) = command("state") else { return EngineState() }
        return EngineState(json: s)
    }

    // MARK: Pixels

    func importImage(_ img: RGBAImage, name: String, asLayer: Bool) throws -> Int64 {
        var err: UnsafeMutablePointer<CChar>?
        let id = img.data.withUnsafeBytes { buf in
            cutout_import_rgba(handle, buf.bindMemory(to: UInt8.self).baseAddress, UInt32(img.width), UInt32(img.height), name, asLayer, &err)
        }
        if id < 0 { throw takeError(err) }
        return id
    }

    /// Render pending changes; returns the frame for tile updates.
    func render() -> CutoutFrame {
        cutout_render(handle)
    }

    func export(unmasked: Bool = false) -> RGBAImage? {
        take(cutout_export_rgba(handle, unmasked))
    }

    func renderUnmasked() -> RGBAImage? {
        take(cutout_render_unmasked(handle))
    }

    func layerPixels(_ id: Int64) -> RGBAImage? {
        take(cutout_layer_pixels(handle, id))
    }

    func thumbnail(_ id: Int64, max: Int) -> RGBAImage? {
        take(cutout_layer_thumbnail(handle, id, UInt32(max)))
    }

    func setLayerMask(_ id: Int64, alpha: [UInt8], width: Int, height: Int, label: String) throws {
        var err: UnsafeMutablePointer<CChar>?
        let ok = alpha.withUnsafeBufferPointer { buf in
            cutout_set_layer_mask(handle, id, buf.baseAddress, UInt32(width), UInt32(height), label, &err)
        }
        if !ok { throw takeError(err) }
    }

    func selectionOutline() -> [Float] {
        let b = cutout_selection_outline(handle)
        defer { cutout_buffer_free(b) }
        guard let p = b.data, b.width > 0 else { return [] }
        return p.withMemoryRebound(to: Float.self, capacity: Int(b.width)) { Array(UnsafeBufferPointer(start: $0, count: Int(b.width))) }
    }

    func textLayer(updateID: Int64, props: TextProps, image: RGBAImage, at point: CGPoint) throws -> Int64 {
        let json = String(data: (try? JSONEncoder().encode(props)) ?? Data(), encoding: .utf8) ?? "{}"
        var err: UnsafeMutablePointer<CChar>?
        let id = image.data.withUnsafeBytes { buf in
            cutout_text_layer(handle, updateID, json, buf.bindMemory(to: UInt8.self).baseAddress, UInt32(image.width), UInt32(image.height), Double(point.x), Double(point.y), &err)
        }
        if id < 0 { throw takeError(err) }
        return id
    }

    // MARK: Helpers

    private func take(_ b: CutoutBuffer) -> RGBAImage? {
        defer { cutout_buffer_free(b) }
        guard let p = b.data, b.len > 0 else { return nil }
        return RGBAImage(width: Int(b.width), height: Int(b.height), data: Data(bytes: p, count: b.len))
    }
}

func takeError(_ p: UnsafeMutablePointer<CChar>?) -> EngineError {
    guard let p else { return EngineError(message: "Unknown error") }
    defer { cutout_string_free(p) }
    return EngineError(message: String(cString: p))
}

/// 8-bit straight-alpha sRGB RGBA image.
struct RGBAImage {
    var width: Int
    var height: Int
    var data: Data

    var hasTransparency: Bool {
        data.withUnsafeBytes { raw in
            let p = raw.bindMemory(to: UInt8.self)
            var i = 3
            while i < p.count {
                if p[i] < 255 { return true }
                i += 4
            }
            return false
        }
    }
}

struct TextProps: Codable, Equatable {
    var text: String
    var font: String
    var size: Double
    var color: [UInt8]
    var bold: Bool
    var italic: Bool
    var align: String
}

struct LayerInfo: Identifiable, Equatable {
    var id: Int64
    var name: String
    var visible: Bool
    var opacity: Double
    var locked: Bool
    var width: Int
    var height: Int
    var hasMask: Bool
    var isText: Bool
    var text: TextProps?
    var transform: LayerTransform
    var bounds: CGRect
}

struct LayerTransform: Equatable {
    var x: Double, y: Double, scaleX: Double, scaleY: Double, rotation: Double

    var json: [String: Any] { ["x": x, "y": y, "scale_x": scaleX, "scale_y": scaleY, "rotation": rotation] }
}

struct EngineState: Equatable {
    var width = 0
    var height = 0
    var layers: [LayerInfo] = []
    var active: Int64?
    var canUndo = false
    var canRedo = false
    var undoLabel: String?
    var redoLabel: String?
    var hasSelection = false
    var selectionBounds: CGRect?
    var modified = false
    var version: Int = 0

    var isEmpty: Bool { layers.isEmpty || width == 0 }
    var activeLayer: LayerInfo? { layers.first { $0.id == active } }

    init() {}

    init(json s: [String: Any]) {
        width = s["width"] as? Int ?? 0
        height = s["height"] as? Int ?? 0
        active = (s["active"] as? NSNumber)?.int64Value
        canUndo = s["can_undo"] as? Bool ?? false
        canRedo = s["can_redo"] as? Bool ?? false
        undoLabel = s["undo_label"] as? String
        redoLabel = s["redo_label"] as? String
        hasSelection = s["has_selection"] as? Bool ?? false
        modified = s["modified"] as? Bool ?? false
        version = s["version"] as? Int ?? 0
        if let b = s["selection_bounds"] as? [String: Any] { selectionBounds = rect(b) }
        layers = (s["layers"] as? [[String: Any]] ?? []).map { l in
            let t = l["transform"] as? [String: Any] ?? [:]
            var text: TextProps?
            if let tj = l["text"], let d = try? JSONSerialization.data(withJSONObject: tj) {
                text = try? JSONDecoder().decode(TextProps.self, from: d)
            }
            return LayerInfo(
                id: (l["id"] as? NSNumber)?.int64Value ?? 0,
                name: l["name"] as? String ?? "",
                visible: l["visible"] as? Bool ?? true,
                opacity: l["opacity"] as? Double ?? 1,
                locked: l["locked"] as? Bool ?? false,
                width: l["width"] as? Int ?? 0,
                height: l["height"] as? Int ?? 0,
                hasMask: l["has_mask"] as? Bool ?? false,
                isText: (l["kind"] as? String) == "text",
                text: text,
                transform: LayerTransform(x: t["x"] as? Double ?? 0, y: t["y"] as? Double ?? 0,
                                          scaleX: t["scale_x"] as? Double ?? 1, scaleY: t["scale_y"] as? Double ?? 1,
                                          rotation: t["rotation"] as? Double ?? 0),
                bounds: rect(l["bounds"] as? [String: Any] ?? [:]) ?? .zero)
        }
    }
}

private func rect(_ d: [String: Any]) -> CGRect? {
    guard let x = d["x"] as? Int, let y = d["y"] as? Int, let w = d["w"] as? Int, let h = d["h"] as? Int else { return nil }
    return CGRect(x: x, y: y, width: w, height: h)
}

/// Background-removal model handle (thread-safe; runs off the main thread).
final class AIModel: @unchecked Sendable {
    private let handle: OpaquePointer

    init(path: URL, useNeuralEngine: Bool, cacheDir: URL) throws {
        var err: UnsafeMutablePointer<CChar>?
        guard let h = cutout_model_load(path.path, useNeuralEngine ? 1 : 0, cacheDir.path, &err) else {
            throw takeError(err)
        }
        handle = h
    }

    deinit { cutout_model_free(handle) }

    func cancel() { cutout_model_cancel(handle) }

    func predict(_ img: RGBAImage, refine: Bool) throws -> [UInt8] {
        var err: UnsafeMutablePointer<CChar>?
        let b = img.data.withUnsafeBytes { buf in
            cutout_model_predict(handle, buf.bindMemory(to: UInt8.self).baseAddress, UInt32(img.width), UInt32(img.height), refine, &err)
        }
        defer { cutout_buffer_free(b) }
        guard let p = b.data, b.len == img.width * img.height else { throw takeError(err) }
        return Array(UnsafeBufferPointer(start: p, count: b.len))
    }
}

/// Edge-aware refinement of an external matte (used for Apple Vision masks).
func refineAlpha(_ img: RGBAImage, alpha: [UInt8], radius: Float) -> [UInt8] {
    let b = img.data.withUnsafeBytes { px in
        alpha.withUnsafeBufferPointer { a in
            cutout_refine_alpha(px.bindMemory(to: UInt8.self).baseAddress, a.baseAddress, UInt32(img.width), UInt32(img.height), radius)
        }
    }
    defer { cutout_buffer_free(b) }
    guard let p = b.data, b.len == alpha.count else { return alpha }
    return Array(UnsafeBufferPointer(start: p, count: b.len))
}
