import AppKit

// Compiled with RenderedText.swift as main.swift, independently of XCTest.
let fixtures = URL(fileURLWithPath: CommandLine.arguments[1])
let size = CGSize(width: 1024, height: 768)
let rendered = try RenderedText.read(Data(contentsOf: fixtures.appendingPathComponent("rendered-output.png")),
                                     displaySize: size)
FileHandle.standardOutput.write(Data(rendered.tsv.utf8))
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
let narrowSize = CGSize(width: 513, height: 680)
let narrow = try RenderedText.read(Data(contentsOf: fixtures.appendingPathComponent("narrow-rendered-output.png")),
                                  displaySize: narrowSize)
FileHandle.standardOutput.write(Data(narrow.tsv.utf8))
guard let narrowOutput = narrow.rect(for: "ZQA 967960 963") else {
    fatalError("The narrow window's rendered nonce must be recognized exactly")
}
precondition(narrow.rect(for: "ZQA 967968 963") == nil, "The observed zero/eight confusion must fail")
precondition(narrow.rect(for: "ZQA 967960 741") == nil, "Wrong narrow-window state must fail")
// Coordinates measured in the original image, relative to the AX window origin.
precondition(abs(narrowOutput.minX - 17) < 4 && abs(narrowOutput.minY - 546) < 4)
precondition(abs(narrowOutput.maxX - 128) < 4 && abs(narrowOutput.maxY - 558) < 4)
let narrowBefore = try RenderedText.read(Data(contentsOf: fixtures.appendingPathComponent("narrow-before-return.png")),
                                        displaySize: narrowSize)
precondition(narrowBefore.rect(for: "ZQA 967960 963") == nil, "Narrow unsubmitted input is not output")

let wideSize = CGSize(width: 1024, height: 678)
let files = try RenderedText.read(Data(contentsOf: fixtures.appendingPathComponent("selected-file-row.png")),
                                 displaySize: wideSize)
FileHandle.standardOutput.write(Data(files.tsv.utf8))
guard let sample = files.rect(for: "SAMPLE") else {
    fatalError("The selected filename must not disappear during segmentation")
}
precondition(files.rect(for: "MISSING") == nil, "An absent filename must fail")
precondition(sample.minX >= 30 && sample.maxX <= 90 && sample.minY >= 212 && sample.maxY <= 237)

let split = try RenderedText.read(Data(contentsOf: fixtures.appendingPathComponent("split-right-output.png")),
                                 displaySize: wideSize)
FileHandle.standardOutput.write(Data(split.tsv.utf8))
guard let left = split.rect(for: "ZQA 285052 741"), let right = split.rect(for: "ZQA 221710 963") else {
    fatalError("Both panes must retain their independent output")
}
precondition(left.maxX < 512 && right.minX > 512, "Output must map to the correct pane")
precondition(split.rect(for: "ZQA 285052 963") == nil, "The other pane's state must not be substituted")
precondition(split.rect(for: "ZQA 221710 741") == nil, "Pane states must remain distinct")
let closeTab = try RenderedText.read(fixtureImage(fixtures, "close-tab-screen.png"), displaySize: wideSize)
FileHandle.standardOutput.write(Data(closeTab.tsv.utf8))
guard let closeTabOutput = closeTab.rect(for: "ZQA 799030 963") else {
    fatalError("The new shell's rendered nonce must be recognized exactly")
}
precondition(closeTab.rect(for: "ZQA 7990308 963") == nil, "The observed extra OCR digit must fail")
precondition(closeTab.rect(for: "ZQA 799030 741") == nil, "A different shell state must fail")
precondition(abs(closeTabOutput.minX - 17) < 4 && abs(closeTabOutput.minY - 546) < 4)
precondition(abs(closeTabOutput.maxX - 128) < 4 && abs(closeTabOutput.maxY - 558) < 4)
let closeTabBefore = try RenderedText.read(fixtureImage(fixtures, "close-tab-before-screen.png"),
                                          displaySize: wideSize)
precondition(closeTabBefore.rect(for: "ZQA 799030 963") == nil, "The new unsubmitted input is not output")
precondition(closeTab.rect(for: "Keyboard shortcuts") == nil, "A terminal is not the settings page")

let settings = try RenderedText.read(fixtureImage(fixtures, "settings-screen.png"), displaySize: wideSize)
FileHandle.standardOutput.write(Data(settings.tsv.utf8))
guard let shortcuts = settings.rect(for: "Keyboard shortcuts") else {
    fatalError("The visible settings navigation label must be recognized exactly")
}
precondition(settings.rect(for: "Keyboard shortcut") == nil, "A partial label must fail")
// Measured in the original screen image, subtracting its recorded AX origin.
precondition(shortcuts.minX >= 30 && shortcuts.maxX <= 140 && shortcuts.minY >= 386 && shortcuts.maxY <= 405)
let tabSetup = try RenderedText.read(fixtureImage(fixtures, "tab-setup-screen.png"), displaySize: wideSize)
FileHandle.standardOutput.write(Data(tabSetup.tsv.utf8))
guard let tabOutput = tabSetup.rect(for: "ZQA 490663 963") else {
    fatalError("The tab setup's rendered nonce must be recognized exactly")
}
precondition(tabSetup.rect(for: "ZQA 499663 963") == nil, "The observed zero/nine confusion must fail")
precondition(tabSetup.rect(for: "ZQA 490663 741") == nil, "A different tab shell state must fail")
// Bounds measured in the unchanged full-screen image, minus its AX window origin.
precondition(abs(tabOutput.minX - 17) < 4 && abs(tabOutput.minY - 546) < 4)
precondition(abs(tabOutput.maxX - 128) < 4 && abs(tabOutput.maxY - 558) < 4)
let tabBefore = try RenderedText.read(fixtureImage(fixtures, "tab-setup-before-screen.png"),
                                     displaySize: wideSize)
precondition(tabBefore.rect(for: "ZQA 490663 963") == nil, "Tab setup input is not executed output")
print("OCR regression: exact output, settings navigation, selected filename, split panes, negative controls and window coordinates passed")
