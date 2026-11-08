import AppKit
import CoreText

/// Rasterises editable text layers with CoreText (the platform text service
/// for the engine's Text layers).
enum TextRenderer {
    static func font(_ p: TextProps) -> NSFont {
        let base = NSFont(name: p.font, size: p.size) ?? NSFont.systemFont(ofSize: p.size)
        var traits: NSFontDescriptor.SymbolicTraits = []
        if p.bold { traits.insert(.bold) }
        if p.italic { traits.insert(.italic) }
        let desc = base.fontDescriptor.withSymbolicTraits(base.fontDescriptor.symbolicTraits.union(traits))
        return NSFont(descriptor: desc, size: p.size) ?? base
    }

    static func render(_ p: TextProps) -> RGBAImage? {
        let text = p.text.isEmpty ? " " : p.text
        let para = NSMutableParagraphStyle()
        para.alignment = p.align == "center" ? .center : (p.align == "right" ? .right : .left)
        let c = p.color.map { CGFloat($0) / 255 }
        let attrs: [NSAttributedString.Key: Any] = [
            .font: font(p),
            .foregroundColor: NSColor(srgbRed: c[0], green: c[1], blue: c[2], alpha: c.count > 3 ? c[3] : 1),
            .paragraphStyle: para,
        ]
        let str = NSAttributedString(string: text, attributes: attrs)
        let bounds = str.boundingRect(with: CGSize(width: 20_000, height: 20_000), options: [.usesLineFragmentOrigin, .usesFontLeading])
        let pad = ceil(p.size * 0.15)
        let w = Int(ceil(bounds.width + pad * 2)), h = Int(ceil(bounds.height + pad * 2))
        guard w > 0, h > 0, w <= 16_384, h <= 16_384 else { return nil }
        let cs = CGColorSpace(name: CGColorSpace.sRGB)!
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: cs,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        let gc = NSGraphicsContext(cgContext: ctx, flipped: false)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = gc
        ctx.setShouldSmoothFonts(false)
        str.draw(with: CGRect(x: pad, y: pad, width: bounds.width, height: bounds.height), options: [.usesLineFragmentOrigin, .usesFontLeading])
        NSGraphicsContext.restoreGraphicsState()
        guard let cg = ctx.makeImage() else { return nil }
        return ImageCodec.rgba(from: cg)
    }
}
