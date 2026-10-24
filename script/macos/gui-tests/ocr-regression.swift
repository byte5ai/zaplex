import AppKit

// Compiled with RenderedText.swift as main.swift, independently of XCTest.
let fixtures = URL(fileURLWithPath: CommandLine.arguments[1])
let size = CGSize(width: 1024, height: 768)
let rendered = try RenderedText.read(Data(contentsOf: fixtures.appendingPathComponent("rendered-output.png")),
                                     displaySize: size)
print(rendered.tsv)
guard let output = rendered.rect(for: "ZQA 149060 963") else {
    fatalError("The saved rendered nonce must be recognized exactly")
}
precondition(rendered.rect(for: "ZQA 149960 963") == nil, "Wrong nonce must fail")
precondition(rendered.rect(for: "ZQA 149060 741") == nil, "Wrong shell state must fail")
// Measured from the original screenshot, not from the OCR implementation.
precondition(abs(output.minX - 17) < 4 && abs(output.minY - 576) < 4)
precondition(abs(output.maxX - 128) < 4 && abs(output.maxY - 588) < 4)
let before = try RenderedText.read(Data(contentsOf: fixtures.appendingPathComponent("before-return.png")),
                                   displaySize: size)
precondition(before.rect(for: "ZQA 149060 963") == nil, "An unsubmitted command is not output")
// A Retina window maps the same pixel image to half as many screen points.
let retina = try RenderedText.read(Data(contentsOf: fixtures.appendingPathComponent("rendered-output.png")),
                                   displaySize: CGSize(width: 512, height: 384))
guard let retinaOutput = retina.rect(for: "ZQA 149060 963") else {
    fatalError("Retina mapping must preserve exact text")
}
precondition(abs(retinaOutput.midX - 36.25) < 2 && abs(retinaOutput.midY - 291) < 2)
print("OCR regression: exact output, negative controls and window coordinates passed")
