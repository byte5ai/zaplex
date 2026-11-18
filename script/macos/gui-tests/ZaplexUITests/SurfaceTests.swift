import AppKit
import XCTest

final class SurfaceTests: GUIFlowTestCase {
    func testWindowMinimumSizePreservesOriginalShell() throws {
        command("qa_state=741")
        try output(state: 741)
        let originalFrame = window.frame
        let original = originalFrame.size
        // A tiled window may extend behind the Dock, but AppKit caps a resize gesture
        // at the screen's visible area, so that is the largest size a user can restore.
        let visible = try XCTUnwrap(NSScreen.screens.first).visibleFrame.size
        let restorable = CGSize(width: min(original.width, visible.width),
                                height: min(original.height, visible.height))
        XCTAssertGreaterThan(restorable.height, 420, "Visible area too small to restore: \(visible)")
        // Resize from the top-left corner: the bottom-right edge stays fixed, so the
        // restore target is the original origin and never lies under the Dock.
        let origin = window.coordinate(withNormalizedOffset: .zero)
        let corner = origin.withOffset(CGVector(dx: 2, dy: 2))
        // Drive AppKit's resize handle with XCTest's explicit pointer API. Hold at the
        // target before releasing: a mouse-up in the same instant as the last drag event
        // can end the live resize before AppKit has applied that final position.
        corner.hover()
        corner.click(forDuration: 0.2, thenDragTo: origin.withOffset(
            CGVector(dx: original.width - 100, dy: original.height - 100)),
                     withVelocity: .default, thenHoldForDuration: 0.5)
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
        let isRestored = {
            abs(self.window.frame.width - restorable.width) < 5 &&
                abs(self.window.frame.height - restorable.height) < 5
        }
        // A synthetic drag can stop a few to 25 points short of its target, even with a
        // hold before mouse-up. Like a user, correct from the corner's current position;
        // a window that cannot grow back stays short and fails below.
        for _ in 0..<3 where !isRestored() {
            let currentFrame = window.frame
            let currentOrigin = window.coordinate(withNormalizedOffset: .zero)
            let currentCorner = currentOrigin.withOffset(CGVector(dx: 2, dy: 2))
            currentCorner.hover()
            currentCorner.click(forDuration: 0.2, thenDragTo: currentOrigin.withOffset(CGVector(
                dx: originalFrame.minX - currentFrame.minX + 2,
                dy: originalFrame.minY - currentFrame.minY + 2)),
                                withVelocity: .slow, thenHoldForDuration: 0.5)
            _ = XCTWaiter.wait(for: [XCTNSPredicateExpectation(
                predicate: NSPredicate { _, _ in isRestored() }, object: nil)], timeout: 2)
        }
        let restored = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in isRestored() }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [restored], timeout: 5), .completed,
                       "Native resize gesture did not restore \(restorable) of \(original), got \(window.frame.size)")
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
