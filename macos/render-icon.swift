import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

/// Opaque 1024×1024 app icon. Drawn with CoreGraphics only so the Dock
/// gets a real image instead of an empty rounded placeholder.
let size = 1024
guard CommandLine.arguments.count > 1 else {
    fputs("usage: render-icon.swift OUTPUT.png\n", stderr)
    exit(1)
}

let colorSpace = CGColorSpaceCreateDeviceRGB()
guard let ctx = CGContext(
    data: nil,
    width: size,
    height: size,
    bitsPerComponent: 8,
    bytesPerRow: 0,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    fputs("could not create bitmap\n", stderr)
    exit(1)
}

let canvas = CGRect(x: 0, y: 0, width: size, height: size)
ctx.setFillColor(CGColor(red: 0.10, green: 0.04, blue: 0.06, alpha: 1))
ctx.fill(canvas)

let backdrop = CGGradient(
    colorsSpace: colorSpace,
    colors: [
        CGColor(red: 0.98, green: 0.34, blue: 0.44, alpha: 1),
        CGColor(red: 0.62, green: 0.08, blue: 0.20, alpha: 1),
        CGColor(red: 0.14, green: 0.05, blue: 0.08, alpha: 1),
    ] as CFArray,
    locations: [0.0, 0.46, 1.0]
)!
ctx.drawLinearGradient(
    backdrop,
    start: CGPoint(x: 140, y: 960),
    end: CGPoint(x: 880, y: 60),
    options: [.drawsBeforeStartLocation, .drawsAfterEndLocation]
)

let glow = CGGradient(
    colorsSpace: colorSpace,
    colors: [
        CGColor(red: 1, green: 0.82, blue: 0.84, alpha: 0.45),
        CGColor(red: 1, green: 0.45, blue: 0.52, alpha: 0),
    ] as CFArray,
    locations: [0, 1]
)!
ctx.drawRadialGradient(
    glow,
    startCenter: CGPoint(x: 512, y: 700),
    startRadius: 8,
    endCenter: CGPoint(x: 512, y: 640),
    endRadius: 440,
    options: []
)

let shell = CGColor(red: 0.98, green: 0.98, blue: 0.99, alpha: 1)
let cushion = CGColor(red: 0.18, green: 0.08, blue: 0.11, alpha: 1)

func cup(_ rect: CGRect) {
    let outer = CGPath(
        roundedRect: rect,
        cornerWidth: rect.width * 0.42,
        cornerHeight: rect.width * 0.42,
        transform: nil
    )
    ctx.addPath(outer)
    ctx.setFillColor(shell)
    ctx.fillPath()

    let inset = rect.insetBy(dx: 26, dy: 34)
    let inner = CGPath(
        roundedRect: inset,
        cornerWidth: inset.width * 0.42,
        cornerHeight: inset.width * 0.42,
        transform: nil
    )
    ctx.addPath(inner)
    ctx.setFillColor(cushion)
    ctx.fillPath()
}

ctx.saveGState()
ctx.setShadow(
    offset: CGSize(width: 0, height: -16),
    blur: 28,
    color: CGColor(red: 0.2, green: 0.02, blue: 0.05, alpha: 0.4)
)
cup(CGRect(x: 228, y: 280, width: 168, height: 250))
cup(CGRect(x: 628, y: 280, width: 168, height: 250))

ctx.setStrokeColor(shell)
ctx.setLineWidth(64)
ctx.setLineCap(.round)
ctx.addArc(
    center: CGPoint(x: 512, y: 478),
    radius: 228,
    startAngle: .pi - 0.18,
    endAngle: 0.18,
    clockwise: true
)
ctx.strokePath()
ctx.restoreGState()

guard let image = ctx.makeImage() else {
    fputs("could not encode image\n", stderr)
    exit(1)
}
let url = URL(fileURLWithPath: CommandLine.arguments[1]) as CFURL
guard let dest = CGImageDestinationCreateWithURL(url, UTType.png.identifier as CFString, 1, nil) else {
    fputs("could not create png\n", stderr)
    exit(1)
}
CGImageDestinationAddImage(dest, image, nil)
if !CGImageDestinationFinalize(dest) {
    fputs("could not write png\n", stderr)
    exit(1)
}
