import AppKit
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

/// Image decoding / encoding via ImageIO. All decoded images are converted to
/// 8-bit straight-alpha sRGB with EXIF orientation applied, so the engine
/// always sees upright pixels in a single colour space (Display P3
/// screenshots are colour-converted, not reinterpreted).
enum ImageCodec {
    static let maxDimension = 16_384
    static let maxPixels = 120_000_000
    static let maxFileBytes = 1 << 30

    static let readableTypes: [UTType] = [.png, .jpeg, .heic, .heif, .tiff, .gif, .bmp, .webP, .image]

    struct DecodeResult {
        var image: RGBAImage
        var downscaled: Bool
    }

    static func decode(url: URL) throws -> DecodeResult {
        let attrs = try? FileManager.default.attributesOfItem(atPath: url.path)
        if let size = attrs?[.size] as? Int, size > maxFileBytes {
            throw EngineError(message: "This file is larger than 1 GB.")
        }
        guard let src = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary) else {
            throw EngineError(message: "This file isn't an image CutOut can read.")
        }
        return try decode(source: src)
    }

    static func decode(data: Data) throws -> DecodeResult {
        guard let src = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) else {
            throw EngineError(message: "The clipboard image could not be read.")
        }
        return try decode(source: src)
    }

    static func decode(source src: CGImageSource) throws -> DecodeResult {
        guard CGImageSourceGetCount(src) > 0,
              let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any] else {
            throw EngineError(message: "The image is empty or corrupted.")
        }
        let pw = props[kCGImagePropertyPixelWidth] as? Int ?? 0
        let ph = props[kCGImagePropertyPixelHeight] as? Int ?? 0
        guard pw > 0, ph > 0 else { throw EngineError(message: "The image has no pixels.") }
        // Decode with orientation applied; downsample huge images up front
        // (thumbnail API decodes at reduced size without a full-size buffer).
        let tooBig = pw > maxDimension || ph > maxDimension || pw * ph > maxPixels
        var maxSide = max(pw, ph)
        if tooBig {
            let s = min(Double(maxDimension) / Double(max(pw, ph)), (Double(maxPixels) / Double(pw * ph)).squareRoot())
            maxSide = Int(Double(max(pw, ph)) * s)
        }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxSide,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else {
            throw EngineError(message: "The image could not be decoded.")
        }
        guard let rgba = rgba(from: cg) else { throw EngineError(message: "The image could not be converted.") }
        return DecodeResult(image: rgba, downscaled: tooBig)
    }

    /// Draw any CGImage into straight-alpha sRGB RGBA.
    static func rgba(from cg: CGImage) -> RGBAImage? {
        let w = cg.width, h = cg.height
        guard w > 0, h > 0, w <= maxDimension, h <= maxDimension else { return nil }
        let cs = CGColorSpace(name: CGColorSpace.sRGB)!
        var buf = Data(count: w * h * 4)
        let ok = buf.withUnsafeMutableBytes { raw -> Bool in
            guard let ctx = CGContext(data: raw.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                                      space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            ctx.interpolationQuality = .none
            ctx.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
            // un-premultiply
            let p = raw.bindMemory(to: UInt8.self)
            var i = 0
            while i < p.count {
                let a = Int(p[i + 3])
                if a > 0 && a < 255 {
                    p[i] = UInt8(min(255, (Int(p[i]) * 255 + a / 2) / a))
                    p[i + 1] = UInt8(min(255, (Int(p[i + 1]) * 255 + a / 2) / a))
                    p[i + 2] = UInt8(min(255, (Int(p[i + 2]) * 255 + a / 2) / a))
                }
                i += 4
            }
            return true
        }
        return ok ? RGBAImage(width: w, height: h, data: buf) : nil
    }

    /// Straight RGBA → CGImage (non-premultiplied, sRGB) suitable for encoding.
    static func cgImage(_ img: RGBAImage, premultiplied: Bool = false) -> CGImage? {
        guard let provider = CGDataProvider(data: img.data as CFData) else { return nil }
        let info: CGImageAlphaInfo = premultiplied ? .premultipliedLast : .last
        return CGImage(width: img.width, height: img.height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: img.width * 4,
                       space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGBitmapInfo(rawValue: info.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }

    /// Flatten onto an opaque colour (for JPEG).
    static func flattened(_ img: RGBAImage, onto bg: (UInt8, UInt8, UInt8) = (255, 255, 255)) -> RGBAImage {
        var out = img
        out.data.withUnsafeMutableBytes { raw in
            let p = raw.bindMemory(to: UInt8.self)
            var i = 0
            while i < p.count {
                let a = Int(p[i + 3])
                if a < 255 {
                    p[i] = UInt8((Int(p[i]) * a + Int(bg.0) * (255 - a) + 127) / 255)
                    p[i + 1] = UInt8((Int(p[i + 1]) * a + Int(bg.1) * (255 - a) + 127) / 255)
                    p[i + 2] = UInt8((Int(p[i + 2]) * a + Int(bg.2) * (255 - a) + 127) / 255)
                    p[i + 3] = 255
                }
                i += 4
            }
        }
        return out
    }

    enum Format: String, CaseIterable, Identifiable {
        case png, jpeg
        var id: String { rawValue }
        var utType: UTType { self == .png ? .png : .jpeg }
        var label: String { self == .png ? "PNG" : "JPEG" }
        var ext: String { self == .png ? "png" : "jpg" }
    }

    static func encode(_ img: RGBAImage, as format: Format, quality: Double = 0.92) -> Data? {
        let src = format == .jpeg ? flattened(img) : img
        guard let cg = cgImage(src) else { return nil }
        return encode(cg, type: format.utType, quality: quality)
    }

    static func encode(_ cg: CGImage, type: UTType, quality: Double = 0.92) -> Data? {
        let data = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(data, type.identifier as CFString, 1, nil) else { return nil }
        var props: [CFString: Any] = [:]
        if type == .jpeg { props[kCGImageDestinationLossyCompressionQuality] = quality }
        CGImageDestinationAddImage(dest, cg, props as CFDictionary)
        guard CGImageDestinationFinalize(dest) else { return nil }
        return data as Data
    }

    static func resized(_ img: RGBAImage, width: Int, height: Int) -> RGBAImage? {
        guard let cg = cgImage(premultiply(img), premultiplied: true) else { return nil }
        let cs = CGColorSpace(name: CGColorSpace.sRGB)!
        guard let ctx = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: cs,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        ctx.interpolationQuality = .high
        ctx.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))
        guard let out = ctx.makeImage() else { return nil }
        return rgba(from: out)
    }

    static func premultiply(_ img: RGBAImage) -> RGBAImage {
        var out = img
        out.data.withUnsafeMutableBytes { raw in
            let p = raw.bindMemory(to: UInt8.self)
            var i = 0
            while i < p.count {
                let a = Int(p[i + 3])
                if a < 255 {
                    p[i] = UInt8((Int(p[i]) * a + 127) / 255)
                    p[i + 1] = UInt8((Int(p[i + 1]) * a + 127) / 255)
                    p[i + 2] = UInt8((Int(p[i + 2]) * a + 127) / 255)
                }
                i += 4
            }
        }
        return out
    }
}
