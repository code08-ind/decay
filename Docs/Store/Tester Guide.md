# Decay beta — tester guide

> Sent to each tester with the unlisted install link. Fill in the links in angle brackets before sending.

Thanks for testing Decay! It takes about 3–4 weeks, and almost all of that is just browsing normally.

**Install:** <Chrome Web Store link> (Chrome 120 or newer; also Brave. In Edge, first turn on "Allow extensions from other stores".)
**Feedback form:** <Google Form link>
**Questions or problems:** gargaryan82000@gmail.com
**Privacy:** https://www.aryangarg.com/decay/privacy-policy. In short, everything stays in your browser.

## Timeline

| When | What to do | Time |
| --- | --- | --- |
| Day 0 | Install. Click **Allow checks** in the popup if you're comfortable with it (it lets Decay see whether a page has unsaved typing before archiving it). Pin the Decay icon to the toolbar. | 2 min |
| First 7 active days | Browse as usual. Decay is in **shadow mode**: it closes nothing and notes what it *would* archive. Only days you browse for at least an hour count, so this can take more than a week. | 0 |
| When shadow mode is done | Open the popup → **Review what Decay would do**. Look through the list, mark anything it got wrong ("Always keep" a site), then **Turn on automatic mode** (called **Turn on automatic mode early** before day 7) → **Turn on**. Note the date. | 5 min |
| The next 14 days | Keep automatic mode on and browse as usual. When you miss a tab, press **Alt+Shift+D**, type a few letters, press Enter. After a sweep, **Undo** in the popup or on the Sweeps page (the clock icon, "Sweep history") brings everything back. | 0 |
| 14 days after turning on automatic mode | Insights → Beta goals shows "Automatic mode: On for N days". At 14 or more, open **Insights** (chart icon in the popup) → **Share with beta team** → **Save file**, then submit the feedback form and attach the file. | 5 min |
| After | Optional 20-minute call about how it went. | 20 min |

If something goes wrong (a tab you needed disappears, or Decay feels too eager), you can switch back to shadow mode at any time: open **Shadow review** (from the Sweeps or Insights page header) → **Back to shadow mode**. To bring a tab back, use the popup search or **Sweeps → Undo**. Please still tell me about it; that's the most useful feedback.

**If you decide to stop, please save the metrics file first** (Insights → Share with beta team → Save file) and send it with the form. Uninstalling Decay deletes everything it stored, including the numbers.

Decay always keeps pinned tabs, tabs playing audio, and sign-in and video-call sites.

## Shortcuts

- **Alt+Shift+D** opens the search popup. If it doesn't work (another extension may use it), set it at chrome://extensions/shortcuts (edge://extensions/shortcuts in Edge).
- **Enter** restores the selected tab into the current window; **Ctrl+Enter** (⌘+Enter on a Mac) restores the whole batch it was archived with, in a new window and in the original order.
- **Esc** clears the search; press it again to close the popup.

## What's in the metrics file

Counts and medians only: the dates you browsed and for how many minutes, open-tab and window counts, numbers of archives, restores, undos and searches, how long search-to-restore took, your Decay settings, how long shadow and automatic mode have run, the extension version and browser name, and a random install id. It contains **no addresses, page titles or site names**, and you see the whole file before saving it.

## What I'm measuring

The beta decides whether Decay becomes a full desktop app. It passes if, after 14 days of automatic mode:
- at least 30% of testers still have automatic mode on;
- at most 15% of archived tabs get reopened within a day (the "regret rate"); and
- at least 5 testers would pre-order the paid desktop version.

So please be honest. "I turned it off on day 3 because…" is exactly what I need to hear.
