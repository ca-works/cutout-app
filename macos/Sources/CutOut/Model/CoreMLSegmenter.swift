#if canImport(CCutOut)
import CCutOut
#endif
import CoreML
import Foundation

/// BiRefNet converted to a Core ML ML Program (tools/convert). Runs on the
/// Neural Engine / GPU; pre- and post-processing are the Rust functions used
/// by the ONNX path (`cutout_ai_preprocess` / `cutout_ai_postprocess`, which
/// includes the guided-filter refinement), so both paths produce the same
/// matte from the same network output.
///
/// Deliberately free of app types so `bench/coreml_bench.swift` can compile
/// this file on its own. Thread-safe; call `predict` off the main thread.
final class CoreMLSegmenter: @unchecked Sendable {
    struct Failure: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    let side = 1024
    private let model: MLModel
    private let lock = NSLock()
    private var cancelled = false
    let loadSeconds: Double

    /// `url` may be an `.mlpackage` (compiled once into `cacheDir`) or an
    /// already compiled `.mlmodelc`.
    init(url: URL, cacheDir: URL, units: MLComputeUnits = .all) throws {
        let t0 = Date()
        let compiled = try Self.compiledURL(for: url, cacheDir: cacheDir)
        let cfg = MLModelConfiguration()
        cfg.computeUnits = units
        model = try MLModel(contentsOf: compiled, configuration: cfg)
        loadSeconds = Date().timeIntervalSince(t0)
    }

    /// Compile an .mlpackage once and keep the .mlmodelc in the cache,
    /// keyed by the package's weight file size + modification date.
    static func compiledURL(for url: URL, cacheDir: URL) throws -> URL {
        if url.pathExtension == "mlmodelc" { return url }
        let fm = FileManager.default
        let weights = url.appendingPathComponent("Data/com.apple.CoreML/weights/weight.bin")
        let attrs = try? fm.attributesOfItem(atPath: weights.path)
        let size = (attrs?[.size] as? NSNumber)?.int64Value ?? 0
        let mtime = Int((attrs?[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0)
        let dest = cacheDir.appendingPathComponent("\(url.deletingPathExtension().lastPathComponent)-\(size)-\(mtime).mlmodelc")
        if fm.fileExists(atPath: dest.appendingPathComponent("coremldata.bin").path) { return dest }
        try fm.createDirectory(at: cacheDir, withIntermediateDirectories: true)
        let tmp = try MLModel.compileModel(at: url)
        try? fm.removeItem(at: dest)
        try fm.moveItem(at: tmp, to: dest)
        // drop compiled copies of older versions of this package (≈450 MB each)
        let prefix = url.deletingPathExtension().lastPathComponent + "-"
        for old in (try? fm.contentsOfDirectory(at: cacheDir, includingPropertiesForKeys: nil)) ?? []
        where old.lastPathComponent.hasPrefix(prefix) && old.lastPathComponent != dest.lastPathComponent {
            try? fm.removeItem(at: old)
        }
        return dest
    }

    /// Best-effort: the current Core ML request cannot be interrupted, but
    /// the result is discarded and `predict` throws "Cancelled".
    func cancel() {
        lock.lock(); cancelled = true; lock.unlock()
    }

    private func checkCancelled() throws {
        lock.lock(); defer { lock.unlock() }
        if cancelled { cancelled = false; throw Failure(message: "Cancelled") }
    }

    /// Low-resolution probability map (side×side, 0..1) for straight RGBA.
    func probabilities(rgba: UnsafePointer<UInt8>, width: Int, height: Int) throws -> [Float] {
        let n = side * side
        let input = try MLMultiArray(shape: [1, 3, NSNumber(value: side), NSNumber(value: side)], dataType: .float32)
        let ok = input.withUnsafeMutableBufferPointer(ofType: Float.self) { buf, strides in
            strides == [3 * n, n, side, 1]
                && cutout_ai_preprocess(rgba, UInt32(width), UInt32(height), UInt32(side), UInt32(side), buf.baseAddress)
        }
        guard ok else { throw Failure(message: "Could not prepare the image for Core ML.") }
        try checkCancelled()
        let features = try MLDictionaryFeatureProvider(dictionary: ["input": MLFeatureValue(multiArray: input)])
        let result = try model.prediction(from: features)
        try checkCancelled()
        guard let out = result.featureValue(for: "alpha")?.multiArrayValue, out.count == n else {
            throw Failure(message: "Unexpected Core ML output.")
        }
        var probs = [Float](repeating: 0, count: n)
        switch out.dataType {
        case .float32:
            out.withUnsafeBufferPointer(ofType: Float.self) { src in
                copyPlane(src.baseAddress!, strides: out.strides.map(\.intValue), into: &probs) { $0 }
            }
        #if arch(arm64)
        case .float16:
            // (MLShapedArrayScalar conformance of Float16 needs macOS 15)
            let src = out.dataPointer.assumingMemoryBound(to: Float16.self)
            copyPlane(UnsafePointer(src), strides: out.strides.map(\.intValue), into: &probs) { Float($0) }
        #endif
        default:
            throw Failure(message: "Unexpected Core ML output type.")
        }
        return probs
    }

    private func copyPlane<T>(_ src: UnsafePointer<T>, strides: [Int], into dst: inout [Float], _ conv: (T) -> Float) {
        let rowStride = strides[strides.count - 2], colStride = strides[strides.count - 1]
        for y in 0..<side {
            for x in 0..<side { dst[y * side + x] = conv(src[y * rowStride + x * colStride]) }
        }
    }

    /// Full-resolution 0..255 matte.
    func predict(rgba: UnsafePointer<UInt8>, width: Int, height: Int, refine: Bool) throws -> [UInt8] {
        lock.lock(); cancelled = false; lock.unlock()
        let probs = try probabilities(rgba: rgba, width: width, height: height)
        let b = probs.withUnsafeBufferPointer {
            cutout_ai_postprocess($0.baseAddress, UInt32(side), UInt32(side), rgba, UInt32(width), UInt32(height), refine)
        }
        defer { cutout_buffer_free(b) }
        guard let p = b.data, b.len == width * height else { throw Failure(message: "Post-processing failed.") }
        return Array(UnsafeBufferPointer(start: p, count: b.len))
    }
}
