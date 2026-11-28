---
layout: ../../layouts/Doc.astro
title: Browsers and the extension
description: Which browsers Decay works with, and how the extension connects to the app.
---

# Browsers and the extension

Decay works with **Google Chrome, Microsoft Edge, Brave, Arc, Chromium** and **Firefox** (128 or later) through a small extension. Chromium browsers install it from the [Chrome Web Store](https://chromewebstore.google.com/detail/jidljehgkacgjoefbancbihmdkkpcaaa). The Firefox version is **in review at Firefox Add-ons** and will be linked here as soon as it's approved. Safari isn't supported.

## Connecting

Click **Connect to the Decay app** in the extension's popup, once per browser profile. The extension then hands its tabs to the app, which does the archiving for every browser at once. If the app isn't running, the extension keeps working on its own.

Each profile is tracked separately, so tabs come back in the profile they came from. Name your profiles in Settings → Browsers to tell them apart.

## Permissions the extension asks for

- **Tabs, windows, storage, alarms, idle, notifications:** to see and age your tabs, keep its data, and show the Undo notification.
- **Access to websites (optional):** to check a page for unsaved typing before archiving it. Without it, pages simply get twice as long.
- **Connect to the Decay app (optional, "communicate with cooperating native applications"):** only asked for when you click Connect.
