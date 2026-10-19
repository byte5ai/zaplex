import AppKit
import XCTest

// The installed release app is deliberately not a build target of this project.
final class SmokeTests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication(url: URL(fileURLWithPath: "/Applications/Zaplex.app"))
        app.launch()
        XCTAssertTrue(app.windows.firstMatch.waitForExistence(timeout: 30), "No window after launch")
        evidence("launch")
    }

    override func tearDownWithError() throws {
        if let app = app, app.state != .notRunning {
            evidence("final-state")
            app.terminate()
        }
    }

    private func evidence(_ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
        let tree = XCTAttachment(string: app.debugDescription)
        tree.name = "\(name)-accessibility"
        tree.lifetime = .keepAlways
        add(tree)
    }

    private func screenText() throws -> String {
        // Vision's text models require an unavailable ANE backend in this VM,
        // even with usesCPUOnly. Tesseract reads the same rendered pixels on CPU.
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/opt/homebrew/bin/tesseract")
        process.arguments = ["stdin", "stdout", "-l", "eng", "--psm", "11"]
        let input = Pipe()
        let output = Pipe()
        process.standardInput = input
        process.standardOutput = output
        process.standardError = output
        try process.run()
        input.fileHandleForWriting.write(app.screenshot().pngRepresentation)
        try input.fileHandleForWriting.close()
        let text = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0, "OCR failed: \(text)")
        return text
    }

    private func expectOutput(_ suffix: String) throws {
        // The full expected string never appears in the typed command. Seeing an
        // unexecuted input line or its echo cannot satisfy this assertion.
        let expected = "ZAPLEX QA \(suffix)"
        app.typeText("printf 'ZAPLEX QA %s\\n' '\(suffix)'\n")
        let deadline = Date().addingTimeInterval(20)
        var visible = ""
        repeat {
            visible = try screenText()
            if visible.contains(expected) {
                evidence("output-\(suffix)")
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.5))
        } while Date() < deadline
        let attachment = XCTAttachment(string: visible)
        attachment.name = "last-visible-text"
        attachment.lifetime = .keepAlways
        add(attachment)
        evidence("missing-output")
        XCTFail("Terminal did not render expected output: \(expected)")
    }

    func testTerminalExecutesAndRendersOutput() throws {
        try expectOutput("314159")
    }

    func testNewWindowRunsItsOwnShell() throws {
        let count = app.windows.count
        app.typeKey("n", modifierFlags: .command)
        let opened = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in self.app.windows.count == count + 1 }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [opened], timeout: 15), .completed, "New window did not open")
        try expectOutput("271828")
    }

    func testNativeCloseButtonClosesWindow() throws {
        let count = app.windows.count
        XCTAssertGreaterThan(count, 0)
        let window = app.windows.firstMatch
        let close = window.buttons[XCUIIdentifierCloseWindow]
        XCTAssertTrue(close.waitForExistence(timeout: 5), "Native close button is missing")
        close.click()
        // firstMatch resolves again after closing and can point at a restored window.
        let closed = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in self.app.windows.count == count - 1 }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 10), .completed, "Close button had no effect")
        evidence("after-native-close")
    }

    func testRelaunchAcceptsTerminalInput() throws {
        app.terminate()
        app.launch()
        XCTAssertTrue(app.windows.firstMatch.waitForExistence(timeout: 30))
        try expectOutput("161803")
    }
}
