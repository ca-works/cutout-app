// Composite RGBA cutouts over a checkerboard and downscale, for visual review.
// Usage: swift scripts/preview_cutout.swift <out.png> <max px> <in1.png> [in2.png ...]  (tiles side by side)
import AppKit
let args = CommandLine.arguments
let out = args[1], maxPx = CGFloat(Double(args[2])!)
let imgs = args.dropFirst(3).compactMap { NSImage(contentsOfFile: $0) }
let cellH = maxPx
let widths = imgs.map { $0.size.width / $0.size.height * cellH }
let W = widths.reduce(0, +) + CGFloat(imgs.count - 1) * 8
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(W), pixelsHigh: Int(cellH), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
NSColor.white.setFill(); NSRect(x: 0, y: 0, width: W, height: cellH).fill()
var x: CGFloat = 0
for (i, img) in imgs.enumerated() {
    let r = NSRect(x: x, y: 0, width: widths[i], height: cellH)
    let c: CGFloat = 10
    var yy: CGFloat = 0; var row = 0
    while yy < cellH { var xx = r.minX + (row % 2 == 0 ? 0 : c); while xx < r.maxX { NSColor(white: 0.8, alpha: 1).setFill(); NSRect(x: xx, y: yy, width: min(c, r.maxX - xx), height: c).fill(); xx += 2 * c }; yy += c; row += 1 }
    img.draw(in: r)
    x += widths[i] + 8
}
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
