// Benchmarks Apple Vision foreground masks (CutOut "Fast" mode) on bench/all.
// Usage: swift bench/vision_bench.swift bench/all bench/out
import AppKit
import Vision

let inDir = URL(fileURLWithPath: CommandLine.arguments[1])
let outDir = URL(fileURLWithPath: CommandLine.arguments[2])
try? FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)

func load(_ u: URL) -> CGImage? {
    guard let src = CGImageSourceCreateWithURL(u as CFURL, nil) else { return nil }
    return CGImageSourceCreateImageAtIndex(src, 0, nil)
}

func gray(_ cg: CGImage) -> [UInt8] {
    let w = cg.width, h = cg.height
    var buf = [UInt8](repeating: 0, count: w * h)
    let ctx = CGContext(data: &buf, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w, space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
    ctx.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
    return buf
}

var rows: [[String: Any]] = []
let items = (try? FileManager.default.contentsOfDirectory(at: inDir, includingPropertiesForKeys: nil))?.sorted { $0.path < $1.path } ?? []
for item in items {
    var isDir: ObjCBool = false
    FileManager.default.fileExists(atPath: item.path, isDirectory: &isDir)
    let imgURL = isDir.boolValue ? item.appendingPathComponent("image.png") : item
    let gtURL: URL? = isDir.boolValue ? item.appendingPathComponent("gt.png") : nil
    guard let cg = load(imgURL) else { continue }
    let name = item.deletingPathExtension().lastPathComponent
    let t0 = Date()
    let req = VNGenerateForegroundInstanceMaskRequest()
    let handler = VNImageRequestHandler(cgImage: cg)
    var alpha = [UInt8](repeating: 0, count: cg.width * cg.height)
    do {
        try handler.perform([req])
        if let obs = req.results?.first, !obs.allInstances.isEmpty {
            let pb = try obs.generateScaledMaskForImage(forInstances: obs.allInstances, from: handler)
            CVPixelBufferLockBaseAddress(pb, .readOnly)
            let base = CVPixelBufferGetBaseAddress(pb)!, rb = CVPixelBufferGetBytesPerRow(pb)
            for y in 0..<cg.height {
                let row = base.advanced(by: y * rb).assumingMemoryBound(to: Float.self)
                for x in 0..<cg.width { alpha[y * cg.width + x] = UInt8(max(0, min(255, (row[x] * 255).rounded()))) }
            }
            CVPixelBufferUnlockBaseAddress(pb, .readOnly)
        }
    } catch { print("  \(name): \(error)") }
    let ms = Int(Date().timeIntervalSince(t0) * 1000)
    var row: [String: Any] = ["image": name, "ms": ms, "w": cg.width, "h": cg.height]
    if let g = gtURL, let gcg = load(g) {
        let gt = gray(gcg)
        var mae = 0.0, inter = 0.0, uni = 0.0
        for i in 0..<gt.count {
            mae += abs(Double(alpha[i]) - Double(gt[i])) / 255
            let p = alpha[i] >= 128, q = gt[i] >= 128
            if p && q { inter += 1 }
            if p || q { uni += 1 }
        }
        row["mae"] = (mae / Double(gt.count) * 10000).rounded() / 10000
        row["iou"] = (inter / max(1, uni) * 10000).rounded() / 10000
    }
    // save cutout
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: cg.width, pixelsHigh: cg.height, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bitmapFormat: .alphaNonpremultiplied, bytesPerRow: cg.width * 4, bitsPerPixel: 32)!
    let ctx = CGContext(data: rep.bitmapData, width: cg.width, height: cg.height, bitsPerComponent: 8, bytesPerRow: cg.width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
    ctx.draw(cg, in: CGRect(x: 0, y: 0, width: cg.width, height: cg.height))
    for i in 0..<(cg.width * cg.height) { rep.bitmapData![i * 4 + 3] = alpha[i] }
    try? rep.representation(using: .png, properties: [:])?.write(to: outDir.appendingPathComponent("\(name)__vision.png"))
    print("  ", row)
    rows.append(row)
}
let d = try! JSONSerialization.data(withJSONObject: ["model": "apple-vision", "results": rows], options: [.prettyPrinted])
try! d.write(to: outDir.appendingPathComponent("report_apple-vision.json"))
