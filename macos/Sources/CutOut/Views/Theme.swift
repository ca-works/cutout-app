import AppKit
import SwiftUI

/// User-selectable appearance. "System" follows macOS.
enum AppearancePreference: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }
    var title: String { rawValue.capitalized }
    var symbol: String {
        switch self {
        case .system: "circle.lefthalf.filled"
        case .light: "sun.max"
        case .dark: "moon"
        }
    }
    var scheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}

/// Design tokens. Every colour is adaptive and tuned separately for light and
/// dark (not simply inverted): the canvas is a quiet neutral that lets both the
/// checkerboard and the artwork read clearly.
enum Theme {
    private static func adaptive(light: NSColor, dark: NSColor) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
        })
    }

    /// Workspace behind the image.
    static let canvas = adaptive(light: NSColor(white: 0.93, alpha: 1), dark: NSColor(white: 0.105, alpha: 1))
    /// Raised card surface (option groups, empty state).
    static let card = adaptive(light: NSColor(white: 1, alpha: 0.72), dark: NSColor(white: 1, alpha: 0.055))
    static let cardStroke = adaptive(light: NSColor(white: 0, alpha: 0.08), dark: NSColor(white: 1, alpha: 0.09))
    static let hover = adaptive(light: NSColor(white: 0, alpha: 0.06), dark: NSColor(white: 1, alpha: 0.09))
    static let selection = Color.accentColor
    /// The wordmark: fully opaque (the label colours are slightly translucent, which would show
    /// where the tightly tracked letters overlap). Matches the website's #f5f5f7 on dark.
    static let wordmark = adaptive(light: NSColor(srgbRed: 0.114, green: 0.114, blue: 0.122, alpha: 1),
                                   dark: NSColor(srgbRed: 0.961, green: 0.961, blue: 0.969, alpha: 1))

    static let cardRadius: CGFloat = 12
}

extension View {
    /// Rounded, softly outlined surface used for grouped controls.
    func cardSurface(radius: CGFloat = Theme.cardRadius) -> some View {
        background(RoundedRectangle(cornerRadius: radius, style: .continuous).fill(Theme.card))
            .overlay(RoundedRectangle(cornerRadius: radius, style: .continuous).strokeBorder(Theme.cardStroke, lineWidth: 1))
    }

    /// Animates `value` changes unless the user prefers reduced motion.
    func motion<V: Equatable>(_ animation: Animation = .smooth(duration: 0.25), value: V) -> some View {
        modifier(MotionModifier(animation: animation, value: value))
    }
}

private struct MotionModifier<V: Equatable>: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let animation: Animation
    let value: V
    func body(content: Content) -> some View {
        content.animation(reduceMotion ? nil : animation, value: value)
    }
}

/// Hover + press feedback for icon-style toolbar-ish buttons.
struct SoftButtonStyle: ButtonStyle {
    @State private var hover = false
    var radius: CGFloat = 7
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(RoundedRectangle(cornerRadius: radius, style: .continuous)
                .fill(configuration.isPressed ? Theme.hover.opacity(1.6) : (hover ? Theme.hover : .clear)))
            .scaleEffect(configuration.isPressed ? 0.96 : 1)
            .onHover { hover = $0 }
            .animation(.snappy(duration: 0.12), value: configuration.isPressed)
            .animation(.easeOut(duration: 0.12), value: hover)
    }
}
