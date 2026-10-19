import XCTest

final class SurfaceTests: GUIFlowTestCase {
    func testNarrowFileManagerKeepsAllActionsVisible() throws {
        // docs/ui/README.md requires all eight labelled actions on narrow panes.
        command("qa_state=741")
        selectMenu("Window", "Move & Resize")
        let left = app.menuItems["Left"]
        XCTAssertTrue(left.waitForExistence(timeout: 5))
        left.click()
        let resized = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in self.window.frame.width <= 700 }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [resized], timeout: 5), .completed,
                       "Window did not become narrow")
        app.typeKey("e", modifierFlags: [.command, .shift])
        for action in ["Rename", "View", "Edit", "Copy", "Move", "Folder", "Delete", "Terminal"] {
            let rect = try visible(action)
            XCTAssertTrue(CGRect(origin: .zero, size: window.frame.size).contains(rect),
                          "Action outside window: \(action)")
        }
        evidence("file-manager-narrow-all-actions")
        app.typeKey("e", modifierFlags: [.command, .shift])
        try output(state: 741)
    }

    func testFileManagerUsesPaneDirectoryAndReturnsToShell() throws {
        command("qa_dir=$(mktemp -d /tmp/zaplex-gui.XXXXXX); cd \"$qa_dir\"; printf 'fixture\\n' > SAMPLE; qa_state=741")
        try output(state: 741)
        app.typeKey("e", modifierFlags: [.command, .shift])
        try visible("SAMPLE")
        try visible("Modified")
        try visible("Copy")
        try visible("Move")
        evidence("file-manager-directory-and-actions")
        app.typeKey("e", modifierFlags: [.command, .shift])
        try output(state: 741)
    }

    func testConnectionsPanelKeepsTerminalUsable() throws {
        command("qa_state=741")
        app.typeKey("5", modifierFlags: .control)
        try visible("No SSH servers yet.")
        evidence("connections-empty-state")
        app.typeKey("5", modifierFlags: .control)
        try output(state: 741)
    }
}
