import CoreGraphics
import Foundation
let opts: CGWindowListOption = [.optionAll, .excludeDesktopElements]
let list = (CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]]) ?? []
let normal = list.filter { ($0[kCGWindowLayer as String] as? Int) == 0 }
print("windows (layer 0):", normal.count, "of", list.count)
var withName = 0
for w in normal.prefix(400) where w[kCGWindowName as String] != nil { withName += 1 }
print("with a title (kCGWindowName):", withName)
print("keys:", Set(normal.flatMap { $0.keys }).sorted())
print("onscreen:", normal.filter { ($0[kCGWindowIsOnscreen as String] as? Bool) == true }.count)
print("preflight screen capture:", CGPreflightScreenCaptureAccess())
