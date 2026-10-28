import AppKit

// Saved-image diagnosis only: no app launch, XCTest run or acceptance verdict.
let fixtures = URL(fileURLWithPath: CommandLine.arguments[1])
let images: [(String, CGSize)] = [
    ("rendered-output.png", CGSize(width: 1024, height: 768)),
    ("before-return.png", CGSize(width: 1024, height: 768)),
    ("selected-file-row.png", CGSize(width: 1024, height: 678)),
    ("narrow-rendered-output.png", CGSize(width: 513, height: 680)),
    ("narrow-before-return.png", CGSize(width: 513, height: 680)),
    ("split-right-output.png", CGSize(width: 1024, height: 678)),
    ("close-tab-screen.png", CGSize(width: 1024, height: 678)),
    ("close-tab-before-screen.png", CGSize(width: 1024, height: 678)),
    ("settings-screen.png", CGSize(width: 1024, height: 678)),
    ("tab-setup-screen.png", CGSize(width: 1024, height: 678)),
    ("tab-setup-before-screen.png", CGSize(width: 1024, height: 678)),
]
for scale in [2, 3, 4, 6] {
    for (name, interpolation) in [("nearest", CGInterpolationQuality.none), ("high", .high)] {
        for segmentation in [6, 11] {
            let configuration = RenderedText.Configuration(scale: scale, interpolation: interpolation,
                                                           pageSegmentation: segmentation)
            for (file, size) in images {
                let reading = try RenderedText.read(fixtureImage(fixtures, file),
                                                    displaySize: size, configuration: configuration)
                let record: [String: Any] = ["file": file, "scale": scale, "interpolation": name,
                                           "psm": segmentation, "tsv": reading.tsv]
                var encoded = try JSONSerialization.data(withJSONObject: record, options: [.sortedKeys])
                encoded.append(10)
                FileHandle.standardOutput.write(encoded)
            }
        }
    }
}
