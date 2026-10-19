import XCTest

final class SurfaceTests: GUIFlowTestCase {
    func testFileManagerUsesPaneDirectoryAndReturnsToShell() throws {
        app.typeText("qa_dir=$(mktemp -d /tmp/zaplex-gui.XXXXXX); cd \"$qa_dir\"; printf 'fixture\\n' > SAMPLE; qa_state=741\n")
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
        app.typeText("qa_state=741\n")
        app.typeKey("5", modifierFlags: .control)
        try visible("Connections")
        evidence("connections-empty-state")
        app.typeKey("5", modifierFlags: .control)
        try output(state: 741)
    }
}
