import AppKit
import XCTest

// Expectations come from GH-562 and the approved pane launch design, not app internals.
class GUIFlowTestCase: XCTestCase {
    var app: XCUIApplication!
    var window: XCUIElement { app.windows.firstMatch }
    private var lastOCR = ""

    private struct Word {
        let text: String
        let rect: CGRect
        let line: String
    }

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication(url: URL(fileURLWithPath: "/Applications/Zaplex.app"))
        app.launch()
        XCTAssertTrue(window.waitForExistence(timeout: 30))
        // Restored tabs from another case must not affect this case's workspace.
        app.typeKey("n", modifierFlags: .command)
        try output(state: 963)
    }

    override func tearDownWithError() throws {
        if let app = app, app.state != .notRunning {
            evidence("final-state")
            app.terminate()
        }
    }

    func evidence(_ name: String, text: String? = nil) {
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = name
        screenshot.lifetime = .keepAlways
        add(screenshot)
        let tree = XCTAttachment(string: text ?? app.debugDescription)
        tree.name = "\(name)-details"
        tree.lifetime = .keepAlways
        add(tree)
    }

    private func words() throws -> [Word] {
        let screenshot = window.screenshot().pngRepresentation
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: screenshot))
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/opt/homebrew/bin/tesseract")
        process.arguments = ["stdin", "stdout", "-l", "eng", "--psm", "11", "tsv"]
        let input = Pipe()
        let output = Pipe()
        process.standardInput = input
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        try process.run()
        input.fileHandleForWriting.write(screenshot)
        try input.fileHandleForWriting.close()
        let tsv = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        lastOCR = tsv
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0, "Tesseract failed")
        let scaleX = window.frame.width / CGFloat(bitmap.pixelsWide)
        let scaleY = window.frame.height / CGFloat(bitmap.pixelsHigh)
        return tsv.split(separator: "\n").compactMap { row in
            let c = row.split(separator: "\t", omittingEmptySubsequences: false)
            guard c.count == 12, c[0] == "5", !c[11].isEmpty,
                  let x = Double(c[6]), let y = Double(c[7]),
                  let w = Double(c[8]), let h = Double(c[9]) else { return nil }
            return Word(text: String(c[11]),
                        rect: CGRect(x: x * scaleX, y: y * scaleY, width: w * scaleX, height: h * scaleY),
                        line: c[1...4].joined(separator: ":"))
        }
    }

    @discardableResult
    func visible(_ phrase: String, timeout: TimeInterval = 15) throws -> CGRect {
        let expected = phrase.split(separator: " ").map(String.init)
        let deadline = Date().addingTimeInterval(timeout)
        var last: [Word] = []
        repeat {
            last = try words()
            for line in Dictionary(grouping: last, by: { $0.line }).values {
                guard line.count >= expected.count else { continue }
                for start in 0...(line.count - expected.count) {
                    let match = Array(line[start..<(start + expected.count)])
                    if match.map({ $0.text }) == expected {
                        return match.dropFirst().reduce(match[0].rect) { $0.union($1.rect) }
                    }
                }
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        } while Date() < deadline
        evidence("missing-visible-content", text: last.map({ $0.text }).joined(separator: " ") + "\n\n" + lastOCR)
        XCTFail("Expected rendered text: \(phrase)")
        throw NSError(domain: "ZaplexGUI", code: 1, userInfo: [NSLocalizedDescriptionKey: phrase])
    }

    @discardableResult
    func output(state: Int) throws -> CGRect {
        let marker = Int.random(in: 100000...999999)
        // The nonce is new for each assertion; old scrollback cannot satisfy it.
        // Non-exported state distinguishes an existing shell from a new one.
        app.typeText("printf 'ZQA %s %s\\n' \(marker) \"${qa_state:-963}\"\n")
        let rect = try visible("ZQA \(marker) \(state)")
        evidence("shell-state-\(state)")
        return rect
    }

    func selectMenu(_ menu: String, _ item: String) {
        app.menuBars.menuBarItems[menu].click()
        let target = app.menuItems[item]
        XCTAssertTrue(target.waitForExistence(timeout: 5), "Menu item missing: \(item)")
        target.click()
    }

    func clickVisible(_ phrase: String) throws {
        // Coordinates are derived from a fresh screenshot of this window.
        let rect = try visible(phrase)
        window.coordinate(withNormalizedOffset: .zero)
            .withOffset(CGVector(dx: rect.midX, dy: rect.midY)).click()
    }

}

final class NavigationTests: GUIFlowTestCase {
    func testTabsKeepIndependentShells() throws {
        app.typeText("qa_state=741\n")
        try output(state: 741)
        selectMenu("File", "New Terminal Tab")
        try output(state: 963)
        app.typeText("qa_state=852\n")
        selectMenu("Tab", "Switch to Previous Tab")
        try output(state: 741)
        selectMenu("Tab", "Switch to Next Tab")
        try output(state: 852)
    }

    func testClosingTabPreservesOriginalShell() throws {
        app.typeText("qa_state=741\n")
        selectMenu("File", "New Terminal Tab")
        try output(state: 963)
        selectMenu("Tab", "Close the Current Tab")
        try output(state: 741)
    }

    private func split(_ direction: String) throws {
        app.typeText("qa_state=741\n")
        try output(state: 741)
        selectMenu("Tab", "Split Pane \(direction)")
        evidence("split-launch-menu-\(direction)")
        try clickVisible("Terminal")
        let newOutput = try output(state: 963)
        selectMenu("Tab", "Activate Previous Pane")
        let original = try output(state: 741)
        switch direction {
        case "Right": XCTAssertGreaterThan(newOutput.midX, original.midX + 100)
        case "Left": XCTAssertLessThan(newOutput.midX, original.midX - 100)
        case "Down": XCTAssertGreaterThan(newOutput.midY, original.midY + 100)
        case "Up": XCTAssertLessThan(newOutput.midY, original.midY - 100)
        default: XCTFail("Unspecified direction")
        }
        evidence("split-geometry-\(direction)")
    }

    func testSplitRight() throws { try split("Right") }
    func testSplitLeft() throws { try split("Left") }
    func testSplitDown() throws { try split("Down") }
    func testSplitUp() throws { try split("Up") }

    func testCancelSplitKeepsOriginalShell() throws {
        app.typeText("qa_state=741\n")
        selectMenu("Tab", "Split Pane Right")
        try visible("Terminal")
        evidence("split-before-cancel")
        app.typeKey(.escape, modifierFlags: [])
        try output(state: 741)
        // A cancelled split must not leave an empty pane beside the original.
        selectMenu("Tab", "Activate Next Pane")
        try output(state: 741)
    }

    func testSettingsReturnToOriginalShell() throws {
        app.typeText("qa_state=741\n")
        app.typeKey(",", modifierFlags: .command)
        try visible("Appearance")
        evidence("settings-open")
        app.typeKey(.escape, modifierFlags: [])
        try output(state: 741)
    }
}
