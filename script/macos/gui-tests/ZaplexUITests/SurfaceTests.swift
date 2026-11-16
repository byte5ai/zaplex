import XCTest

final class SurfaceTests: GUIFlowTestCase {
    func testWindowMinimumSizePreservesOriginalShell() throws {
        command("qa_state=741")
        try output(state: 741)
        let originalFrame = window.frame
        let original = originalFrame.size
        // Resize from the top-left corner: the bottom-right edge stays fixed, so the
        // restore target is the original origin and never lies under the Dock, which
        // stops AppKit's resize gesture short of windows that extend behind it.
        let origin = window.coordinate(withNormalizedOffset: .zero)
        let corner = origin.withOffset(CGVector(dx: 2, dy: 2))
        // Drive AppKit's resize handle with XCTest's explicit pointer API.
        corner.hover()
        corner.click(forDuration: 0.2, thenDragTo: origin.withOffset(
            CGVector(dx: original.width - 100, dy: original.height - 100)))
        let resized = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                self.window.frame.width < original.width - 10 ||
                self.window.frame.height < original.height - 10
            },
            object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [resized], timeout: 5), .completed,
                       "Native resize gesture did not shrink the window")
        evidence("window-minimum-size")
        XCTAssertGreaterThanOrEqual(window.frame.width, 480)
        XCTAssertGreaterThanOrEqual(window.frame.height, 320)
        XCTAssertLessThanOrEqual(window.frame.width, 500)
        XCTAssertLessThanOrEqual(window.frame.height, 360)
        try output(state: 741)
        let resizedFrame = window.frame
        let resizedOrigin = window.coordinate(withNormalizedOffset: .zero)
        let resizedCorner = resizedOrigin.withOffset(CGVector(dx: 2, dy: 2))
        resizedCorner.hover()
        resizedCorner.click(forDuration: 0.2, thenDragTo: resizedOrigin.withOffset(CGVector(
            dx: originalFrame.minX - resizedFrame.minX + 2,
            dy: originalFrame.minY - resizedFrame.minY + 2)))
        let restored = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                abs(self.window.frame.width - original.width) < 5 &&
                abs(self.window.frame.height - original.height) < 5
            }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [restored], timeout: 5), .completed,
                       "Native resize gesture did not restore \(original), got \(window.frame.size)")
        try output(state: 741)
        evidence("window-size-restored")
    }

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
        for action in ["Rename", "Details", "Edit", "Copy", "Move", "Folder", "Delete", "Terminal"] {
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
