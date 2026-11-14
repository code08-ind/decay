import AppKit
import ApplicationServices

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let w = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 300, height: 200),
                 styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
w.title = "report.txt"
let f = URL(fileURLWithPath: NSTemporaryDirectory() + "decay-spike-report.txt")
try? "x".write(to: f, atomically: true, encoding: .utf8)
w.representedURL = f
w.isDocumentEdited = true
w.orderFront(nil)

func names(_ e: AXUIElement) -> [String] {
    var arr: CFArray?
    let err = AXUIElementCopyAttributeNames(e, &arr)
    if err != .success { return ["<err \(err.rawValue)>"] }
    return (arr as? [String]) ?? []
}
func val(_ e: AXUIElement, _ a: String) -> String {
    var v: CFTypeRef?
    let err = AXUIElementCopyAttributeValue(e, a as CFString, &v)
    if err != .success { return "<err \(err.rawValue)>" }
    return String(describing: v!)
}
DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
    let me = AXUIElementCreateApplication(getpid())
    print("trusted:", AXIsProcessTrusted())
    print("app attrs:", names(me))
    var wins: CFTypeRef?
    let e = AXUIElementCopyAttributeValue(me, kAXWindowsAttribute as CFString, &wins)
    print("windows err:", e.rawValue)
    for win in (wins as? [AXUIElement]) ?? [] {
        print("window attrs:", names(win))
        for a in ["AXTitle", "AXDocument", "AXMinimized", "AXEdited", "AXModified", "AXSubrole", "AXIdentifier"] {
            print("  \(a) =", val(win, a))
        }
        var cb: CFTypeRef?
        if AXUIElementCopyAttributeValue(win, kAXCloseButtonAttribute as CFString, &cb) == .success {
            let b = cb as! AXUIElement
            print("close button attrs:", names(b))
            for a in ["AXEdited", "AXSubrole", "AXValue", "AXDescription"] { print("  btn \(a) =", val(b, a)) }
        }
        // Minimize via AX, then read back.
        let m = AXUIElementSetAttributeValue(win, kAXMinimizedAttribute as CFString, kCFBooleanTrue)
        print("set AXMinimized err:", m.rawValue)
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
            print("  AXMinimized after =", val(win, "AXMinimized"), "isMiniaturized:", w.isMiniaturized)
            w.isDocumentEdited = false
            print("  after save: close-button AXEdited =", { var cb: CFTypeRef?; AXUIElementCopyAttributeValue(win, kAXCloseButtonAttribute as CFString, &cb); return val(cb as! AXUIElement, "AXEdited") }())
            exit(0)
        }
    }
}
app.run()
