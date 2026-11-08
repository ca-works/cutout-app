// Benchmarks the Core ML BiRefNet (tools/convert) with the app's own
// CoreMLSegmenter and the Rust pre/post-processing. Writes 8-bit alpha
// mattes `<name>__<tag>.png` that `cutout-cli score` compares against ground
// truth and against the ONNX outputs. Build + run: bench/run_coreml.sh
import CoreGraphics
import CoreML
import Foundation
import ImageIO
import UniformTypeIdentifiers

func loadRGBA(_ url: URL) -> (data: [UInt8], w: Int, h: Int)? {
    guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
          let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else { return nil }
    let w = img.width, h = img.height
    // Prefer the decoded bytes untouched (no colour management), as the Rust
    // `image` crate reads them.
    if img.bitsPerComponent == 8, img.bitsPerPixel == 8, img.alphaInfo == .none, let d = img.dataProvider?.data as Data? {
        var out = [UInt8](repeating: 255, count: w * h * 4)   // greyscale
        d.withUnsafeBytes { (p: UnsafeRawBufferPointer) in
            for y in 0..<h { for x in 0..<w { let v = p[y * img.bytesPerRow + x], o = (y * w + x) * 4; out[o] = v; out[o + 1] = v; out[o + 2] = v } }
        }
        return (out, w, h)
    }
    if img.bitsPerComponent == 8, let d = img.dataProvider?.data as Data?,
       img.bitsPerPixel == 24 || img.bitsPerPixel == 32 {
        let bpp = img.bitsPerPixel / 8, row = img.bytesPerRow
        let alpha = img.alphaInfo
        let alphaFirst = alpha == .first || alpha == .premultipliedFirst || alpha == .noneSkipFirst
        let hasAlpha = alpha == .last || alpha == .first || alpha == .premultipliedLast || alpha == .premultipliedFirst
        let premul = alpha == .premultipliedLast || alpha == .premultipliedFirst
        if img.byteOrderInfo == .orderDefault || img.byteOrderInfo == .order32Big {
            var out = [UInt8](repeating: 255, count: w * h * 4)
            d.withUnsafeBytes { (p: UnsafeRawBufferPointer) in
                for y in 0..<h {
                    for x in 0..<w {
                        let s = y * row + x * bpp, o = (y * w + x) * 4
                        let c0 = (bpp == 4 && alphaFirst) ? 1 : 0
                        var a: UInt8 = 255
                        if bpp == 4 && hasAlpha { a = p[alphaFirst ? s : s + 3] }
                        for c in 0..<3 {
                            var v = Int(p[s + c0 + c])
                            if premul && a > 0 && a < 255 { v = min(255, (v * 255 + Int(a) / 2) / Int(a)) }
                            out[o + c] = UInt8(v)
                        }
                        out[o + 3] = a
                    }
                }
            }
            return (out, w, h)
        }
    }
    return nil
}

func writeGray(_ a: [UInt8], w: Int, h: Int, to url: URL) {
    let provider = CGDataProvider(data: Data(a) as CFData)!
    let img = CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 8, bytesPerRow: w,
                      space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.none.rawValue),
                      provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    let dst = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dst, img, nil)
    CGImageDestinationFinalize(dst)
}

func residentMB() -> Double {
    var info = mach_task_basic_info()
    var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size) / 4
    _ = withUnsafeMutablePointer(to: &info) { $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count) } }
    return Double(info.resident_size_max) / 1_048_576
}

let args = CommandLine.arguments
guard args.count >= 4 else {
    print("usage: coreml_bench <model.mlpackage|.mlmodelc> <images dir> <out dir> [all|ane|gpu|cpu] [--no-refine]")
    exit(2)
}
let unitsName = args.count > 4 && !args[4].hasPrefix("--") ? args[4] : "all"
let units: MLComputeUnits = ["ane": .cpuAndNeuralEngine, "gpu": .cpuAndGPU, "cpu": .cpuOnly][unitsName] ?? .all
let refine = !args.contains("--no-refine")
let imagesDir = URL(fileURLWithPath: args[2]), outDir = URL(fileURLWithPath: args[3])
try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)
let cache = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("cutout-coreml-bench")
let tag = "birefnet-coreml_\(unitsName)\(refine ? "" : "_noref")"

let t0 = Date()
let seg = try CoreMLSegmenter(url: URL(fileURLWithPath: args[1]), cacheDir: cache, units: units)
print(String(format: "model %@ units %@ load %.0f ms (incl. compile if first use)", args[1], unitsName, Date().timeIntervalSince(t0) * 1000))

var rows: [[String: Any]] = []
var warm = false
let entries = try FileManager.default.contentsOfDirectory(at: imagesDir, includingPropertiesForKeys: nil).sorted { $0.lastPathComponent < $1.lastPathComponent }
for e in entries {
    var isDir: ObjCBool = false
    FileManager.default.fileExists(atPath: e.path, isDirectory: &isDir)
    let imgURL = isDir.boolValue ? e.appendingPathComponent("image.png") : e
    guard ["png", "jpg", "jpeg"].contains(imgURL.pathExtension.lowercased()) else { continue }
    guard let (rgba, w, h) = loadRGBA(imgURL) else { print("  skipped \(imgURL.lastPathComponent): unsupported pixel format"); continue }
    let name = isDir.boolValue ? e.lastPathComponent : e.deletingPathExtension().lastPathComponent
    if !warm {
        let tw = Date()
        _ = try rgba.withUnsafeBufferPointer { try seg.predict(rgba: $0.baseAddress!, width: w, height: h, refine: false) }
        print(String(format: "  warm-up %.0f ms", Date().timeIntervalSince(tw) * 1000))
        warm = true
    }
    let ti = Date()
    let probs = try rgba.withUnsafeBufferPointer { try seg.probabilities(rgba: $0.baseAddress!, width: w, height: h) }
    let inference = Date().timeIntervalSince(ti) * 1000
    let tt = Date()
    let alpha = try rgba.withUnsafeBufferPointer { try seg.predict(rgba: $0.baseAddress!, width: w, height: h, refine: refine) }
    let total = Date().timeIntervalSince(tt) * 1000
    _ = probs
    writeGray(alpha, w: w, h: h, to: outDir.appendingPathComponent("\(name)__\(tag).png"))
    let row: [String: Any] = ["image": name, "w": w, "h": h, "inference_ms": Int(inference), "total_ms": Int(total)]
    print("  \(row)")
    rows.append(row)
}
let report: [String: Any] = ["model": args[1], "units": unitsName, "refine": refine, "peak_rss_mb": Int(residentMB()), "results": rows]
let data = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
try data.write(to: outDir.appendingPathComponent("report_\(tag).json"))
print(String(format: "peak RSS %.0f MB", residentMB()))
