import AppKit
import UniformTypeIdentifiers

/// Native pasteboard integration. CutOut only reads the pasteboard when the
/// user pastes; it never monitors it.
@MainActor
enum Clipboard {
    struct Item {
        var image: RGBAImage
        var name: String
        var downscaled: Bool
    }

    static var hasImage: Bool {
        let pb = NSPasteboard.general
        if pb.canReadObject(forClasses: [NSURL.self], options: urlOptions) { return true }
        return pb.availableType(from: imageTypes) != nil
    }

    private static let urlOptions: [NSPasteboard.ReadingOptionKey: Any] = [
        .urlReadingFileURLsOnly: true,
        .urlReadingContentsConformToTypes: [UTType.image.identifier],
    ]

    private static let imageTypes: [NSPasteboard.PasteboardType] = [
        .png, NSPasteboard.PasteboardType(UTType.heic.identifier), .tiff,
        NSPasteboard.PasteboardType(UTType.jpeg.identifier), NSPasteboard.PasteboardType(UTType.webP.identifier),
        NSPasteboard.PasteboardType(UTType.gif.identifier), .pdf,
    ]

    /// Read every image on the pasteboard (Finder files first, then image data).
    static func read(from pb: NSPasteboard = .general) throws -> [Item] {
        var items: [Item] = []
        if let urls = pb.readObjects(forClasses: [NSURL.self], options: urlOptions) as? [URL], !urls.isEmpty {
            for url in urls.prefix(16) {
                let r = try ImageCodec.decode(url: url)
                items.append(Item(image: r.image, name: url.deletingPathExtension().lastPathComponent, downscaled: r.downscaled))
            }
            return items
        }
        for t in imageTypes where t != .pdf {
            if let d = pb.data(forType: t), let r = try? ImageCodec.decode(data: d) {
                return [Item(image: r.image, name: "Pasted Image", downscaled: r.downscaled)]
            }
        }
        // Last resort (PDF, vendor types): let AppKit render it.
        if let img = NSImage(pasteboard: pb) {
            var rect = CGRect(origin: .zero, size: img.size)
            if let cg = img.cgImage(forProposedRect: &rect, context: nil, hints: nil), let rgba = ImageCodec.rgba(from: cg) {
                return [Item(image: rgba, name: "Pasted Image", downscaled: false)]
            }
        }
        if items.isEmpty { throw EngineError(message: "The clipboard doesn't contain an image.") }
        return items
    }

    /// Write an image with real alpha. PNG is published together with TIFF
    /// (many apps such as Keynote and PowerPoint read TIFF first). Data is
    /// written eagerly — not promised — so it survives CutOut quitting.
    /// Returns only after verifying the pasteboard holds the data.
    static func write(_ img: RGBAImage, format: ImageCodec.Format) throws {
        if SelfTest.failNextClipboardWrite {
            SelfTest.failNextClipboardWrite = false
            throw EngineError(message: "Simulated clipboard failure (self-test).")
        }
        let pb = NSPasteboard.general
        let src = format == .jpeg ? ImageCodec.flattened(img) : img
        guard let cg = ImageCodec.cgImage(src) else { throw EngineError(message: "Could not prepare the image.") }
        guard let main = ImageCodec.encode(cg, type: format.utType),
              let tiff = ImageCodec.encode(cg, type: .tiff) else {
            throw EngineError(message: "Could not encode the image.")
        }
        let mainType = format == .png ? NSPasteboard.PasteboardType.png : NSPasteboard.PasteboardType(UTType.jpeg.identifier)
        pb.clearContents()
        pb.declareTypes([mainType, .tiff], owner: nil)
        guard pb.setData(main, forType: mainType), pb.setData(tiff, forType: .tiff) else {
            throw EngineError(message: "The clipboard rejected the image.")
        }
        // verify
        guard let back = pb.data(forType: mainType), back.count == main.count else {
            throw EngineError(message: "The clipboard could not be verified.")
        }
    }
}
