---
layout: ../../layouts/Doc.astro
title: How Decay decides
description: Active time, the stages a tab goes through, quiet sweeps, and what Decay never touches.
---

# How Decay decides

## Active time, not the clock

Decay ages your tabs and windows only while you're actually using your computer. When it's idle (no input for 5 minutes by default), locked or asleep, nothing ages: a weekend away doesn't make Monday's tabs old. The tab you're looking at, and the window in front, are always "in use".

## The stages

1. **Fresh** while you use it.
2. **Stale** once it's gone unused for the pace's first threshold (shown in the Library; nothing happens to it yet).
3. **Hidden** (app windows only): minimised out of the way. Bring it back and it's fresh again.
4. **Archived**: the tab is closed and kept in the archive, ready to restore. Tabs skip the Hidden stage.

## Quiet sweeps

Decay archives in **sweeps**: at most every 15 minutes, only at a quiet moment (you haven't switched tabs or typed for a short while), at most 50 tabs at a time. Every sweep can be undone (the last 20 are kept in the Library's Sweeps).

Sweeps **wait while you're in a call or a meeting** (a microphone or camera in use) and for two minutes after. They also wait while you've paused Decay or put it on hold (the menu: hold for an hour, until tomorrow, or until you resume).

## What Decay never touches

- Pinned tabs, and tabs playing audio.
- Pages where you're typing, uploading, or that ask "leave site?" (Decay checks right before archiving, with your permission; a page it can't check gets twice as long).
- Sites and apps you mark **Always keep** ([Rules](/docs/rules)).
- Private / incognito windows: never recorded.
- App windows with unsaved changes: they're only ever minimised, never closed.
