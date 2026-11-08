import CoreML
import CryptoKit
import Foundation
import Vision

/// A downloadable background-removal model. URLs are pinned to an exact
/// Hugging Face revision and verified by SHA-256 before installation.
struct ModelSpec: Identifiable, Equatable {
    let id: String
    let title: String
    let detail: String
    let fileName: String
    let url: URL
    let sha256: String
    let bytes: Int64
    let license: String

    var sizeLabel: String { ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file) }

    static let birefnetGeneral = ModelSpec(
        id: "birefnet-general-fp32",
        title: "BiRefNet (Best Quality)",
        detail: "High-resolution dichotomous segmentation. Excellent on hair, fur, thin structures and mechanical parts.",
        fileName: "birefnet-general-fp32.onnx",
        url: URL(string: "https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67/onnx/model.onnx")!,
        sha256: "58f621f00f5d756097615970a88a791584600dcf7c45b18a0a6267535a1ebd3c",
        bytes: 972_666_916,
        license: "MIT (ZhengPeng7/BiRefNet; ONNX export by onnx-community)")

    static let birefnetCompact = ModelSpec(
        id: "birefnet-general-fp16",
        title: "BiRefNet (Best Quality, compact)",
        detail: "The same model stored at half precision: half the download, identical results in our tests, about 15% slower.",
        fileName: "birefnet-general-fp16.onnx",
        url: URL(string: "https://huggingface.co/onnx-community/BiRefNet-ONNX/resolve/534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67/onnx/model_fp16.onnx")!,
        sha256: "3654c741eb80bd926ada8fed1713b506ccf8d30eb1f6487e87eb9f234f33df09",
        bytes: 489_666_272,
        license: "MIT (ZhengPeng7/BiRefNet; ONNX export by onnx-community)")

    static let birefnetLite = ModelSpec(
        id: "birefnet-lite-fp32",
        title: "BiRefNet Lite (Faster)",
        detail: "Smaller variant. Noticeably weaker on enclosed holes and low-contrast edges; not faster on CPU in our tests.",
        fileName: "birefnet-lite-fp32.onnx",
        url: URL(string: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/de15b22ba131738a16dff04aab8bdf8dc32e3ac1/onnx/model.onnx")!,
        sha256: "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333",
        bytes: 224_005_088,
        license: "MIT (ZhengPeng7/BiRefNet; ONNX export by onnx-community)")

    static let all: [ModelSpec] = [.birefnetGeneral, .birefnetCompact, .birefnetLite]
}

/// BiRefNet (general) converted to a Core ML ML Program (tools/convert).
///
/// Where CutOut finds it, in order:
/// 1. `CutOut.app/Contents/Resources/Models/birefnet-general-coreml.mlmodelc`
///    — precompiled and bundled by `BUNDLE_COREML=1 scripts/build.sh`; covered
///    by the app's code signature, ready without any setup or compilation.
/// 2. `~/Library/Application Support/CutOut/Models/birefnet-general-coreml.mlpackage`
///    — installed in-app ("Install from File…", or a download once a hosting
///    location is approved) or by the developer recipe; its weights are
///    SHA-256-verified before use and compiled once into the cache.
///
/// Runs on CPU+GPU (the Neural Engine compiler does not finish on this
/// network; docs/MODELS.md). The ONNX CPU model is the automatic fallback
/// when it is installed.
enum CoreMLBiRefNet {
    static let packageName = "birefnet-general-coreml.mlpackage"
    static let compiledName = "birefnet-general-coreml.mlmodelc"
    /// SHA-256 of Data/com.apple.CoreML/weights/weight.bin produced by
    /// tools/convert/convert.py (see tools/convert/OUTPUT.txt).
    static let weightsSHA256 = "66044c149f7a152d9b1c522a681a09ebb9d73db5fd27fefaf05c79ec9034287f"
    static let weightsBytes: Int64 = 463_618_272
    /// Pre-converted package download. Disabled until a hosting location for
    /// the converted weights is approved (nothing has been published).
    static let download: (url: URL, bytes: Int64, sha256: String)? = nil
    /// The ONNX models that hold the same network.
    static let equivalentSpecs: Set<String> = [ModelSpec.birefnetGeneral.id, ModelSpec.birefnetCompact.id]

    enum Source: Equatable {
        case bundled(URL)
        case installed(URL)
        var url: URL {
            switch self {
            case .bundled(let u), .installed(let u): u
            }
        }
        var label: String {
            switch self {
            case .bundled: "Included with CutOut"
            case .installed: "Installed in Application Support"
            }
        }
    }

    /// Core ML BiRefNet is validated on Apple Silicon only (Intel Macs use ONNX).
    static var isSupportedHardware: Bool {
        #if arch(arm64)
        return true
        #else
        return false
        #endif
    }

    static var computeUnits: MLComputeUnits {
        switch ProcessInfo.processInfo.environment["CUTOUT_COREML_UNITS"] {
        case "all": return .all
        case "ane": return .cpuAndNeuralEngine
        case "cpu": return .cpuOnly
        default: return .cpuAndGPU
        }
    }

    static var unitsLabel: String {
        switch computeUnits {
        case .all: "all units"
        case .cpuAndNeuralEngine: "CPU + Neural Engine"
        case .cpuOnly: "CPU"
        default: "GPU"
        }
    }

    static var isEnabled: Bool {
        if let v = ProcessInfo.processInfo.environment["CUTOUT_COREML"] { return v != "0" }
        return UserDefaults.standard.object(forKey: "useCoreML") as? Bool ?? true
    }

    static var installedPackageURL: URL { ModelManager.modelsDirectory.appendingPathComponent(packageName) }

    static func locate() -> Source? {
        if let r = Bundle.main.resourceURL {
            let c = r.appendingPathComponent("Models/\(compiledName)")
            if FileManager.default.fileExists(atPath: c.appendingPathComponent("coremldata.bin").path) { return .bundled(c) }
            let p = r.appendingPathComponent("Models/\(packageName)")
            if FileManager.default.fileExists(atPath: p.path) { return .bundled(p) }
        }
        let u = installedPackageURL
        return FileManager.default.fileExists(atPath: u.path) ? .installed(u) : nil
    }

    static func weightsURL(_ pkg: URL) -> URL {
        pkg.pathExtension == "mlmodelc" ? pkg.appendingPathComponent("weights/weight.bin")
            : pkg.appendingPathComponent("Data/com.apple.CoreML/weights/weight.bin")
    }

    /// Bundled models are covered by the code signature. Installed packages
    /// must match the pinned conversion output; the result is remembered per
    /// (size, mtime) so the 460 MB hash runs once.
    static func verify(_ source: Source) -> Bool {
        if case .bundled = source { return true }
        let w = weightsURL(source.url)
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: w.path),
              let size = (attrs[.size] as? NSNumber)?.int64Value, size == weightsBytes else { return false }
        let mtime = Int((attrs[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0)
        let key = "coremlVerified:\(size):\(mtime):\(weightsSHA256)"
        if UserDefaults.standard.bool(forKey: key) { return true }
        guard (try? ModelManager.sha256(of: w)) == weightsSHA256 else { return false }
        UserDefaults.standard.set(true, forKey: key)
        return true
    }

    /// Install a converted model from a `.mlpackage` folder or a `.zip`
    /// containing one. Verifies the weights before anything is replaced.
    static func install(from src: URL) throws {
        let fm = FileManager.default
        let work = fm.temporaryDirectory.appendingPathComponent("cutout-install-\(UUID().uuidString)")
        try fm.createDirectory(at: work, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: work) }
        var pkg = src
        if src.pathExtension.lowercased() == "zip" {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
            p.arguments = ["-x", "-k", src.path, work.path]
            try p.run()
            p.waitUntilExit()
            guard p.terminationStatus == 0 else { throw EngineError(message: "The archive could not be extracted.") }
            guard let found = fm.enumerator(at: work, includingPropertiesForKeys: nil)?
                .compactMap({ $0 as? URL }).first(where: { $0.pathExtension == "mlpackage" }) else {
                throw EngineError(message: "The archive does not contain a Core ML model (.mlpackage).")
            }
            pkg = found
        }
        guard pkg.pathExtension == "mlpackage" else { throw EngineError(message: "Choose a .mlpackage folder or a .zip archive.") }
        let w = weightsURL(pkg)
        let size = ((try? fm.attributesOfItem(atPath: w.path))?[.size] as? NSNumber)?.int64Value ?? -1
        guard size == weightsBytes, (try? ModelManager.sha256(of: w)) == weightsSHA256 else {
            throw EngineError(message: "This is not the verified CutOut BiRefNet Core ML model (weights checksum mismatch). Nothing was installed.")
        }
        try fm.createDirectory(at: ModelManager.modelsDirectory, withIntermediateDirectories: true)
        let staged = ModelManager.modelsDirectory.appendingPathComponent(".\(packageName).staging")
        try? fm.removeItem(at: staged)
        try fm.copyItem(at: pkg, to: staged)
        try? fm.removeItem(at: installedPackageURL)
        try fm.moveItem(at: staged, to: installedPackageURL)
    }
}

/// Anything that turns an image into a full-resolution matte.
protocol Segmenter: AnyObject, Sendable {
    func predict(_ img: RGBAImage, refine: Bool) throws -> [UInt8]
    func cancel()
    /// Human-readable backend that produced the last result.
    var backend: String { get }
}

extension AIModel: Segmenter {
    var backend: String { "ONNX Runtime · CPU" }
}

/// Core ML first; on any Core ML failure the ONNX CPU model is loaded (when
/// installed) and used instead, and kept for later calls.
final class CoreMLWithFallback: Segmenter, @unchecked Sendable {
    /// Released as soon as we fall back, so both models are never resident.
    private var coreML: CoreMLSegmenter?
    private let loadFallback: (@Sendable () throws -> AIModel)?
    private let lock = NSLock()
    private var fallback: AIModel?
    private var lastBackend: String

    init(coreML: CoreMLSegmenter, loadFallback: (@Sendable () throws -> AIModel)?) {
        self.coreML = coreML
        self.loadFallback = loadFallback
        lastBackend = "Core ML · \(CoreMLBiRefNet.unitsLabel)"
    }

    var backend: String {
        lock.lock(); defer { lock.unlock() }
        return lastBackend
    }

    func predict(_ img: RGBAImage, refine: Bool) throws -> [UInt8] {
        lock.lock(); let fb = fallback; let core = coreML; lock.unlock()
        if let fb { return try fb.predict(img, refine: refine) }
        guard let core else { throw EngineError(message: "No model loaded.") }
        do {
            if ProcessInfo.processInfo.environment["CUTOUT_COREML_FORCE_FAIL"] == "1" {
                throw EngineError(message: "simulated Core ML failure (test)")
            }
            return try img.data.withUnsafeBytes { buf in
                try core.predict(rgba: buf.bindMemory(to: UInt8.self).baseAddress!, width: img.width, height: img.height, refine: refine)
            }
        } catch let e as CoreMLSegmenter.Failure where e.message == "Cancelled" {
            throw e
        } catch {
            NSLog("CutOut: Core ML inference failed (%@)", error.localizedDescription)
            guard let loadFallback else {
                throw EngineError(message: "Core ML inference failed (\(error.localizedDescription)) and no ONNX fallback model is installed.")
            }
            lock.lock(); coreML = nil; lock.unlock()   // free Core ML memory before loading ONNX
            let m = try loadFallback()
            lock.lock(); fallback = m; lastBackend = "ONNX Runtime · CPU (Core ML failed)"; lock.unlock()
            return try m.predict(img, refine: refine)
        }
    }

    func cancel() {
        lock.lock(); let fb = fallback; let core = coreML; lock.unlock()
        core?.cancel()
        fb?.cancel()
    }
}

enum RemovalMode: String, CaseIterable, Identifiable {
    case best, fast
    var id: String { rawValue }
}

/// Owns model files and the lazily-loaded inference session.
@MainActor
@Observable
final class ModelManager {
    static let shared = ModelManager()

    var downloadProgress: Double?
    var downloadingSpec: ModelSpec?
    var lastError: String?

    private var loaded: (id: String, model: any Segmenter)?
    /// Backend that produced the most recent result (shown in the UI).
    var activeBackend: String?
    /// Why Core ML could not be used in this session, if it failed.
    var coreMLFailure: String?
    private var downloadTask: Task<Void, Never>?

    nonisolated static var modelsDirectory: URL {
        if let o = ProcessInfo.processInfo.environment["CUTOUT_MODELS_DIR"], !o.isEmpty { return URL(fileURLWithPath: o, isDirectory: true) }
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("CutOut/Models", isDirectory: true)
    }

    nonisolated static var coreMLCache: URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("CutOut/coreml", isDirectory: true)
    }

    /// Optional model bundled inside the app (Contents/Resources/Models).
    nonisolated static func bundledURL(_ spec: ModelSpec) -> URL? {
        guard let u = Bundle.main.resourceURL?.appendingPathComponent("Models/\(spec.fileName)"),
              FileManager.default.fileExists(atPath: u.path) else { return nil }
        return u
    }

    func url(for spec: ModelSpec) -> URL? {
        if let b = Self.bundledURL(spec) { return b }
        let u = Self.modelsDirectory.appendingPathComponent(spec.fileName)
        return FileManager.default.fileExists(atPath: u.path) ? u : nil
    }

    func isInstalled(_ spec: ModelSpec) -> Bool { url(for: spec) != nil }

    var preferredSpec: ModelSpec {
        let id = UserDefaults.standard.string(forKey: "bestModel") ?? ModelSpec.birefnetGeneral.id
        return ModelSpec.all.first { $0.id == id } ?? .birefnetGeneral
    }

    var installedBestSpec: ModelSpec? {
        if isInstalled(preferredSpec) { return preferredSpec }
        return ModelSpec.all.first { isInstalled($0) }
    }

    /// Whether Core ML BiRefNet can be used right now (present, enabled,
    /// supported hardware, not failed earlier in this session).
    var coreMLSource: CoreMLBiRefNet.Source? {
        guard CoreMLBiRefNet.isSupportedHardware, CoreMLBiRefNet.isEnabled, coreMLFailure == nil else { return nil }
        return CoreMLBiRefNet.locate()
    }

    /// True if Best Quality can run without any setup.
    var hasBestModel: Bool { coreMLSource != nil || installedBestSpec != nil }

    /// Load (once) and return the best available segmenter: the Core ML
    /// model when present and verified, otherwise the ONNX CPU model.
    /// Loading/compiling happens here, on first use — never at startup.
    func bestSegmenter() async throws -> any Segmenter {
        unloadTask?.cancel()
        if let l = loaded { return l.model }
        let preferLite = preferredSpec.id == ModelSpec.birefnetLite.id && isInstalled(.birefnetLite)
        let onnxSpec = installedBestSpec
        let cache = Self.coreMLCache
        let loadONNX: (@Sendable () throws -> AIModel)? = onnxSpec.flatMap { spec in
            url(for: spec).map { u in { try AIModel(path: u, useNeuralEngine: false, cacheDir: cache) } }
        }
        if !preferLite, let source = coreMLSource {
            let units = CoreMLBiRefNet.computeUnits
            let r: Result<CoreMLWithFallback, Error> = await Task.detached(priority: .userInitiated) {
                guard CoreMLBiRefNet.verify(source) else {
                    return .failure(EngineError(message: "The Core ML model failed verification."))
                }
                do {
                    let core = try CoreMLSegmenter(url: source.url, cacheDir: cache.appendingPathComponent("compiled"), units: units)
                    return .success(CoreMLWithFallback(coreML: core, loadFallback: loadONNX))
                } catch {
                    return .failure(error)
                }
            }.value
            switch r {
            case .success(let seg):
                loaded = ("coreml", seg)
                return seg
            case .failure(let e):
                coreMLFailure = e.localizedDescription
                NSLog("CutOut: Core ML unavailable (%@); falling back to ONNX", e.localizedDescription)
            }
        }
        guard let loadONNX, let spec = onnxSpec else {
            throw EngineError(message: coreMLFailure.map { "Core ML could not be used (\($0)) and no ONNX model is installed." }
                              ?? "No best-quality model is installed.")
        }
        let m = try await Task.detached(priority: .userInitiated) { try loadONNX() }.value
        loaded = (spec.id, m)
        return m
    }

    func unload() { loaded = nil }

    /// Install a converted Core ML model from a file chosen by the user.
    func installCoreML(from url: URL) async {
        lastError = nil
        installingCoreML = true
        defer { installingCoreML = false }
        do {
            try await Task.detached(priority: .userInitiated) { try CoreMLBiRefNet.install(from: url) }.value
            coreMLFailure = nil
            loaded = nil
        } catch {
            lastError = error.localizedDescription
        }
    }

    var installingCoreML = false

    func deleteCoreML() {
        try? FileManager.default.removeItem(at: CoreMLBiRefNet.installedPackageURL)
        loaded = nil
    }

    private var unloadTask: Task<Void, Never>?

    /// Free the model's memory after a period without use.
    func scheduleUnload(after seconds: Double = 240) {
        unloadTask?.cancel()
        unloadTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            self?.loaded = nil
        }
    }

    // MARK: Download (only ever started by an explicit user action)

    func download(_ spec: ModelSpec) {
        guard downloadTask == nil else { return }
        lastError = nil
        downloadingSpec = spec
        downloadProgress = 0
        downloadTask = Task { [weak self] in
            do {
                try await Self.performDownload(spec) { p in
                    Task { @MainActor in self?.downloadProgress = p }
                }
                await MainActor.run {
                    self?.downloadProgress = nil
                    self?.downloadingSpec = nil
                    self?.downloadTask = nil
                }
            } catch {
                await MainActor.run {
                    self?.lastError = (error as? CancellationError) != nil ? nil : error.localizedDescription
                    self?.downloadProgress = nil
                    self?.downloadingSpec = nil
                    self?.downloadTask = nil
                }
            }
        }
    }

    func cancelDownload() {
        downloadTask?.cancel()
    }

    func delete(_ spec: ModelSpec) {
        let u = Self.modelsDirectory.appendingPathComponent(spec.fileName)
        try? FileManager.default.removeItem(at: u)
        if loaded?.id == spec.id { loaded = nil }
    }

    private nonisolated static func performDownload(_ spec: ModelSpec, progress: @escaping @Sendable (Double) -> Void) async throws {
        let dir = modelsDirectory
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let part = dir.appendingPathComponent(spec.fileName + ".part")
        try? FileManager.default.removeItem(at: part)
        let box = TaskBox()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
                let task = URLSession.shared.downloadTask(with: spec.url) { tmp, response, error in
                    if let error { cont.resume(throwing: error); return }
                    guard let tmp, let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                        cont.resume(throwing: EngineError(message: "The download server returned an error.")); return
                    }
                    do {
                        try FileManager.default.moveItem(at: tmp, to: part)
                        cont.resume()
                    } catch { cont.resume(throwing: error) }
                }
                box.observation = task.progress.observe(\.fractionCompleted) { p, _ in progress(p.fractionCompleted * 0.97) }
                box.task = task
                task.resume()
            }
        } onCancel: {
            box.task?.cancel()
        }
        // verify size and SHA-256 by streaming the file
        let attrs = try FileManager.default.attributesOfItem(atPath: part.path)
        guard (attrs[.size] as? Int64 ?? Int64(attrs[.size] as? Int ?? -1)) == spec.bytes,
              try sha256(of: part) == spec.sha256 else {
            try? FileManager.default.removeItem(at: part)
            throw EngineError(message: "The downloaded model failed its integrity check and was discarded.")
        }
        let dest = dir.appendingPathComponent(spec.fileName)
        try? FileManager.default.removeItem(at: dest)
        try FileManager.default.moveItem(at: part, to: dest)
        progress(1)
    }

    nonisolated static func sha256(of url: URL) throws -> String {
        let fh = try FileHandle(forReadingFrom: url)
        defer { try? fh.close() }
        var hasher = SHA256()
        while let chunk = try fh.read(upToCount: 8 << 20), !chunk.isEmpty {
            hasher.update(data: chunk)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }
}

private final class TaskBox: @unchecked Sendable {
    var task: URLSessionDownloadTask?
    var observation: NSKeyValueObservation?
}

/// Apple Vision foreground segmentation (macOS 14+): built in, no download,
/// runs on the Neural Engine. Used for Fast mode.
enum VisionRemover {
    static func alpha(for img: RGBAImage) throws -> [UInt8] {
        guard let cg = ImageCodec.cgImage(ImageCodec.premultiply(img), premultiplied: true) else {
            throw EngineError(message: "Could not prepare the image.")
        }
        let request = VNGenerateForegroundInstanceMaskRequest()
        let handler = VNImageRequestHandler(cgImage: cg, options: [:])
        try handler.perform([request])
        guard let obs = request.results?.first, !obs.allInstances.isEmpty else {
            throw EngineError(message: "No foreground subject was found. Try Best Quality mode or the selection tools.")
        }
        let buffer = try obs.generateScaledMaskForImage(forInstances: obs.allInstances, from: handler)
        CVPixelBufferLockBaseAddress(buffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
        let w = CVPixelBufferGetWidth(buffer), h = CVPixelBufferGetHeight(buffer)
        let rowBytes = CVPixelBufferGetBytesPerRow(buffer)
        guard w == img.width, h == img.height, let base = CVPixelBufferGetBaseAddress(buffer) else {
            throw EngineError(message: "Unexpected mask size from Vision.")
        }
        var out = [UInt8](repeating: 0, count: w * h)
        let fmt = CVPixelBufferGetPixelFormatType(buffer)
        for y in 0..<h {
            let row = base.advanced(by: y * rowBytes)
            for x in 0..<w {
                let v: Float
                if fmt == kCVPixelFormatType_OneComponent32Float {
                    v = row.assumingMemoryBound(to: Float.self)[x]
                } else {
                    v = Float(row.assumingMemoryBound(to: UInt8.self)[x]) / 255
                }
                out[y * w + x] = UInt8(max(0, min(255, (v * 255).rounded())))
            }
        }
        return out
    }
}
