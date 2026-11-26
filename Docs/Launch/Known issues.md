# Known issues (pinned: https://github.com/code08-ind/decay-releases/issues/1)

Source of the pinned issue on decay-releases. Edit here, then `gh issue edit 1 -R code08-ind/decay-releases --body-file <this text without the title>`. Remove items as they are fixed (Firefox once AMO lists it; the Mac and Windows warnings once signing exists).

---

Things you may run into with the current version (**0.1.2**), and what to do. Updated as they're fixed.

### macOS: "Apple could not verify Decay…" on first launch
Decay isn't notarized by Apple yet. Click **Done**, then open **System Settings → Privacy & Security**, scroll down and click **Open Anyway** next to Decay. You only do this once. (Right-click → Open no longer works on macOS 15 and later.)

### macOS: a password prompt after an update
After Decay updates itself, macOS may say *"Decay wants to use your confidential information stored in com.decay.app"*. Enter your Mac login password and click **Always Allow**. This is Decay opening its own encrypted database; it happens because the app isn't signed with an Apple Developer certificate yet.

### Windows: "Windows protected your PC" (SmartScreen)
Decay isn't code-signed on Windows yet. Click **More info → Run anyway**.

### The extension says it can't connect to the app
The Chrome Web Store extension needs **Decay 0.1.1 or later**. Decay updates itself within a day; to update now, open Decay's **Settings → Updates → Check now**, or download the latest version from https://decay.aryangarg.com/download. Then click **Connect to the Decay app** in the extension's popup again.

### Firefox
The Firefox add-on is in review at Firefox Add-ons. Until it's listed, Decay works with Chrome, Edge, Brave and Arc through the [Chrome Web Store extension](https://chromewebstore.google.com/detail/jidljehgkacgjoefbancbihmdkkpcaaa) (Edge installs it from there too).

### Something else?
[Open an issue](https://github.com/code08-ind/decay-releases/issues/new) or email **support.decay@aryangarg.com**. Settings → Privacy → **Export diagnostics…** saves a file you can attach (it contains no tab addresses, titles or sites).
