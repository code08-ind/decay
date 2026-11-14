// P2.0 spike: what Accessibility reports for other apps' windows (needs Accessibility
// access for the terminal you run it from: System Settings → Privacy & Security →
// Accessibility). Prints, per regular app: each window's title, document, edited state
// (the close button's AXEdited), minimised / full-screen state and whether it can be
// minimised. Read-only: it changes nothing.
//
//   swiftc -O tools/spikes/ax-other-apps.swift -o /tmp/ax-other-apps && /tmp/ax-other-apps
import AppKit
import ApplicationServices

func val(_ e: AXUIElement, _ a: String) -> CFTypeRef? {
    var v: CFTypeRef?
    return AXUIElementCopyAttributeValue(e, a as CFString, &v) == .success ? v : nil
}

guard AXIsProcessTrusted() else {
    print("Not trusted: grant Accessibility to this terminal first.")
    exit(1)
}
for app in NSWorkspace.shared.runningApplications where app.activationPolicy == .regular {
    let ax = AXUIElementCreateApplication(app.processIdentifier)
    let windows = (val(ax, kAXWindowsAttribute) as? [AXUIElement]) ?? []
    print("\(app.localizedName ?? "?") (\(app.bundleIdentifier ?? "?")): \(windows.count) windows")
    for w in windows {
        let title = val(w, kAXTitleAttribute) as? String ?? ""
        let doc = val(w, "AXDocument") as? String
        let edited = (val(w, kAXCloseButtonAttribute)).flatMap { val($0 as! AXUIElement, "AXEdited") }
        let minimizable = (val(w, kAXMinimizeButtonAttribute)).flatMap { val($0 as! AXUIElement, kAXEnabledAttribute) }
        print("  [\(val(w, kAXSubroleAttribute) ?? "-" as CFString)] \"\(title)\" doc=\(doc ?? "-") edited=\(edited.map { "\($0)" } ?? "unknown") minimized=\(val(w, kAXMinimizedAttribute).map { "\($0)" } ?? "?") fullscreen=\(val(w, "AXFullScreen").map { "\($0)" } ?? "?") canMinimize=\(minimizable.map { "\($0)" } ?? "no button")")
    }
}
