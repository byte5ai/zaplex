import AppKit

// OCR belongs to the test process. Keep exact text matching and map the enlarged
// image's coordinates back to the original window before driving mouse input.
enum RenderedText {
    // Fixed production settings; explicit alternatives are only for saved-image diagnosis.
    struct Configuration {
        let scale: Int
        let interpolation: CGInterpolationQuality
        let pageSegmentation: Int
        static let standard = Configuration(scale: 2, interpolation: .high, pageSegmentation: 11)
    }

    struct Word {
        let text: String
        let rect: CGRect
        let line: String
    }

    struct Reading {
        let words: [Word]
        let tsv: String

        func rect(for phrase: String) -> CGRect? {
            let expected = phrase.split(separator: " ").map(String.init)
            guard !expected.isEmpty else { return nil }
            for line in Dictionary(grouping: words, by: { $0.line }).values {
                guard line.count >= expected.count else { continue }
                for start in 0...(line.count - expected.count) {
                    let match = Array(line[start..<(start + expected.count)])
                    if match.map({ $0.text }) == expected {
                        return match.dropFirst().reduce(match[0].rect) { $0.union($1.rect) }
                    }
                }
            }
            return nil
        }
    }

    static func read(_ png: Data, displaySize: CGSize,
                     configuration: Configuration = .standard) throws -> Reading {
        guard let bitmap = NSBitmapImageRep(data: png), let original = bitmap.cgImage else {
            throw NSError(domain: "ZaplexOCR", code: 1)
        }
        // Small terminal glyphs are only about ten pixels high on the runner.
        // Resample twofold with the fixed interpolation verified against saved
        // positive and negative images. Recognition and comparison remain exact.
        let width = bitmap.pixelsWide * configuration.scale
        let height = bitmap.pixelsHigh * configuration.scale
        guard let context = CGContext(data: nil, width: width, height: height,
                                      bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
            throw NSError(domain: "ZaplexOCR", code: 2)
        }
        context.interpolationQuality = configuration.interpolation
        context.draw(original, in: CGRect(x: 0, y: 0, width: width, height: height))
        guard let image = context.makeImage(),
              let enlarged = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
            throw NSError(domain: "ZaplexOCR", code: 3)
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/opt/homebrew/bin/tesseract")
        // UI labels and terminal output occupy separate blocks. Sparse-text
        // segmentation preserves those regions at the verified twofold scale.
        process.arguments = ["stdin", "stdout", "-l", "eng", "--psm", String(configuration.pageSegmentation), "tsv"]
        let input = Pipe()
        let output = Pipe()
        process.standardInput = input
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        try process.run()
        input.fileHandleForWriting.write(enlarged)
        try input.fileHandleForWriting.close()
        let tsv = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw NSError(domain: "ZaplexOCR", code: Int(process.terminationStatus))
        }
        let scaleX = displaySize.width / CGFloat(width)
        let scaleY = displaySize.height / CGFloat(height)
        let words: [Word] = tsv.split(separator: "\n").compactMap { row in
            let c = row.split(separator: "\t", omittingEmptySubsequences: false)
            guard c.count == 12, c[0] == "5", !c[11].isEmpty,
                  let x = Double(c[6]), let y = Double(c[7]),
                  let w = Double(c[8]), let h = Double(c[9]) else { return nil }
            return Word(text: String(c[11]),
                        rect: CGRect(x: x * scaleX, y: y * scaleY, width: w * scaleX, height: h * scaleY),
                        line: c[1...4].joined(separator: ":"))
        }
        return Reading(words: words, tsv: tsv)
    }
}
