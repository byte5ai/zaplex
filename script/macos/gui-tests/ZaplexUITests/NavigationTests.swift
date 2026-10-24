import AppKit
import XCTest

// Expectations come from GH-562 and the approved pane launch design, not app internals.
class GUIFlowTestCase: XCTestCase {
    var app: XCUIApplication!
    var window: XCUIElement { app.windows.firstMatch }
    private var lastOCR = ""

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication(url: URL(fileURLWithPath: "/Applications/Zaplex.app"))
        app.launch()
        XCTAssertTrue(window.waitForExistence(timeout: 30))
        // Restored tabs from another case must not affect this case's workspace.
        let count = app.windows.count
        app.typeKey("n", modifierFlags: .command)
        let opened = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in self.app.windows.count == count + 1 }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [opened], timeout: 15), .completed)
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

    @discardableResult
    func visible(_ phrase: String, timeout: TimeInterval = 15) throws -> CGRect {
        let deadline = Date().addingTimeInterval(timeout)
        var last: [RenderedText.Word] = []
        repeat {
            let reading = try RenderedText.read(window.screenshot().pngRepresentation,
                                                displaySize: window.frame.size)
            last = reading.words
            lastOCR = reading.tsv
            if let rect = reading.rect(for: phrase) {
                return rect
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
        command("printf 'ZQA %s %s\\n' \(marker) \"${qa_state:-963}\"")
        let rect = try visible("ZQA \(marker) \(state)")
        evidence("shell-state-\(state)")
        return rect
    }

    func command(_ text: String) {
        app.typeText(text)
        // Event synthesis can finish before the editor has applied all text.
        // Synchronize on the public AX value; output still requires rendered pixels.
        let typed = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                (self.window.textViews.firstMatch.value as? String)?.contains(text) == true
            }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [typed], timeout: 10), .completed,
                       "Typed command did not reach the focused editor")
        evidence("before-single-return")
        // Submit exactly once; never hide a lost Return by retrying the command.
        app.typeKey(.return, modifierFlags: [])
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
        command("qa_state=741")
        try output(state: 741)
        selectMenu("File", "New Terminal Tab")
        try output(state: 963)
        command("qa_state=852")
        selectMenu("Tab", "Switch to Previous Tab")
        try output(state: 741)
        selectMenu("Tab", "Switch to Next Tab")
        try output(state: 852)
    }

    func testClosingTabPreservesOriginalShell() throws {
        command("qa_state=741")
        selectMenu("File", "New Terminal Tab")
        try output(state: 963)
        selectMenu("Tab", "Close the Current Tab")
        try output(state: 741)
    }

    private func split(_ direction: String) throws {
        command("qa_state=741")
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
        command("qa_state=741")
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
        command("qa_state=741")
        app.typeKey(",", modifierFlags: .command)
        try visible("Settings")
        try visible("Keyboard shortcuts")
        evidence("settings-open")
        // Settings is a tab, not a modal dialog.
        selectMenu("Tab", "Close the Current Tab")
        try output(state: 741)
    }
}
