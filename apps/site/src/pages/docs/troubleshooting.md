---
layout: ../../layouts/Doc.astro
title: Troubleshooting
description: The extension won't connect, a restore is refused, sweeps don't happen.
---

# Troubleshooting

## The extension says it can't reach the app

Open Decay (it lives in the menu bar / the tray), then click **Connect to the Decay app** in the extension's popup again. If you quit Decay from its menu, the extension won't start it again until you open it yourself.

## "Open Chrome · Work to restore this"

The tab came from a browser profile that isn't open or isn't connected. Open that profile (and connect its extension), or pick "Open in … instead".

## Nothing is being archived

- Shadow mode: the first week only shows what Decay would do (Library → Shadow review). Switch to Automatic when you're ready.
- Paused or on hold: check the menu.
- In a call: sweeps wait while a microphone or camera is in use. An app that keeps the microphone open (dictation, noise filters) holds them too.
- Not enough active time yet: tabs age only while you're using your computer.

## Windows aren't hidden (macOS)

Decay needs Accessibility access (System Settings → Privacy & Security → Accessibility). After an update, macOS may ask again: remove the old Decay entry and add it back.

## Still stuck?

Settings → Privacy & data → **Export diagnostics** saves a file without addresses, titles or sites. [Report the problem](https://github.com/code08-ind/decay-releases/issues) and attach it if you like.
