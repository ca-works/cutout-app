import CoreText
import SwiftUI

/// The CutOut wordmark, set exactly like the website's: Mona Sans at weight 800 and width 125 %,
/// with tight tracking. The font ships in the bundle (`Resources/Fonts`, SIL OFL 1.1); it is
/// loaded straight from its file, so nothing is installed or registered system-wide.
enum BrandFont {
    private static let wght = 0x7767_6874 // 'wght'
    private static let wdth = 0x7764_7468 // 'wdth'

    private static let descriptor: CTFontDescriptor? = {
        guard let url = Bundle.main.url(forResource: "MonaSans-Latin", withExtension: "woff2", subdirectory: "Fonts"),
              let all = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor],
              let first = all.first
        else { return nil }
        return CTFontDescriptorCreateCopyWithAttributes(first, [kCTFontVariationAttribute: [wght: 800, wdth: 125]] as CFDictionary)
    }()

    /// True when the bundled font loaded (false in a bare `swift run`, which uses the fallback).
    static var isAvailable: Bool { descriptor != nil }

    static func wordmark(size: CGFloat) -> Font {
        if let d = descriptor { return Font(CTFontCreateWithFontDescriptor(d, size, nil)) }
        return .system(size: size, weight: .heavy).width(.expanded)
    }
}

/// "CutOut" in the brand face. `size` is the font size in points; the word is about 3.65 × as wide.
struct Wordmark: View {
    var size: CGFloat

    var body: some View {
        Text("CutOut")
            .font(BrandFont.wordmark(size: size))
            .tracking(-0.065 * size)
            // The negative tracking pulls the last letter past the text's frame; re-centre the ink.
            .offset(x: -0.0325 * size)
            .lineLimit(1)
            .fixedSize()
            .foregroundStyle(Theme.wordmark)
            .accessibilityLabel("CutOut")
            .accessibilityAddTraits(.isHeader)
    }
}
