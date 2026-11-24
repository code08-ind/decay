---
layout: ../../layouts/Doc.astro
title: Privacy and your data
description: What Decay stores, where, what it sends, and how to remove it.
---

# Privacy and your data

**What's stored:** the addresses, titles and sites of your tabs; app windows' titles (and folders' locations); when you used each; your rules and settings. Never page or document content. Private / incognito windows are never recorded.

**Where:** one encrypted database on your computer (its key in the macOS Keychain or Windows Credential Manager), with a daily backup (the last 7).

**What's sent:** a daily check for updates (from GitHub); crash reports only if you turn them on (Settings → Privacy & data), with your computer's name, file paths and addresses removed. Nothing else: no account, no analytics.

**What Decay checks without storing:** whether a microphone or camera is in use (so sweeps wait during calls); whether a page has unsaved typing (with your permission).

**Keeping and removing:** Settings → Privacy & data sets how long the archive is kept (30 days to 10 years; more than 90 days needs [Decay Pro](/docs/pro)), exports it (JSON, bookmarks or CSV), imports, and lists the sites you never want recorded. To remove everything, uninstall Decay and delete its data (Windows: tick *Delete the application data*; macOS: delete `~/Library/Application Support/Decay`).

The full [privacy policy](https://www.aryangarg.com/decay/privacy-policy).
