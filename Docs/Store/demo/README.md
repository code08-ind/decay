# Store media

Screenshots and promo tiles: `screenshots.mjs` and `promo.mjs` (commands in `../Store Listing.md`). Playwright: `npm i --no-save playwright` doesn't work inside the pnpm workspace; install it in a folder outside the repo and symlink its `node_modules` here (git-ignored).

## Demo video (store review)

`../assets/decay-demo.mp4` was recorded from the real extension in headless Chromium. `stage.html` / `stage.js` form a recording-only extension page, **never shipped**: a live tab strip read from `chrome.tabs`, a caption, and a Decay page (popup or Sweeps) in a frame.

To re-record after UI changes:

```sh
pnpm --filter @decay/extension build
cp -R apps/extension/.output/chrome-mv3 /tmp/decay-demo-ext
cp Docs/Store/demo/stage.* /tmp/decay-demo-ext/
cd Docs/Store/demo && npm i --no-save playwright && npx playwright install chromium
node record.mjs /tmp/decay-demo-ext      # prints START <s> and VIDEO <path>
ffmpeg -ss <START + 0.5> -i <VIDEO> -vf "crop=1280:712:0:0,fps=30,format=yuv420p" \
  -c:v libx264 -crf 20 -movflags +faststart ../assets/decay-demo.mp4
```

Notes: the three tabs' age is set directly in IndexedDB (the video says so on screen). Tabs are opened as background tabs of one window, because Decay never closes a window's active tab. The video is cropped to 712 px because headless Chromium shrinks the page after a restored tab activates.
