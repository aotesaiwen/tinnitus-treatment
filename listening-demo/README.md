# Listening Room

A small static blind-listening demo. Each browser gets a shuffled order and
anonymous sample letters. The listener records sound clarity, musicality, desire
to keep listening, optional artifacts, and notes before revealing the sources.
Ratings stay in that browser's local storage and can be exported as JSON.

The published Site is owner-private. `.openai/hosting.json` identifies it; reuse
that project when publishing updates. No API keys, model downloads, analytics, or
backend service are needed to listen.

## Preview

From the repository root, with the existing app dependencies installed:

```sh
node app/node_modules/vite/bin/vite.js preview --outDir listening-demo/dist --host 127.0.0.1 --port 8765
```

Use a static server that supports HTTP byte-range requests. A plain Python
`http.server` can play these FLAC files but prevents seeking in Chromium.
Open the printed URL; opening `index.html` directly does not support JSON fetches.

The six short listening excerpts are versioned with the page. Full raw audio and
model weights stay outside Git. To rebuild the excerpts locally, follow
[`research/listening/README.md`](../research/listening/README.md).

## Session behavior

- Starting a round shuffles actual excerpts; reloading preserves that round's
  labels, scores, notes, and position in the sample list.
- One audio element prevents simultaneous playback. Switching can retain the
  playback position. The initial playback volume is 65%.
- A sample needs playback and all three scores, or an explicit skip, to finish.
- Revealing fetches the source map, locks the initial ratings, and enables JSON
  export. A new round clears local ratings after confirmation.
- This is personal listening blinding. Source files remain inspectable by a
  determined visitor; the site is not a secured research randomization service.

## Verification

With the populated preview running:

```sh
node research/listening/check_demo.cjs http://127.0.0.1:8765
```

This exercises actual media playback and seeking, deferred source disclosure,
saved scores, skip/reveal gating, locked scores, literal note rendering, JSON
export, reset, and narrow layouts. It saves screenshots under `.runtime/previews`.

`dist/` is the entire deployable app: HTML, CSS, JavaScript, manifest, reveal map,
and lossless audio. There is no build step. Publish an exact committed snapshot
including the audio using Sites; do not reuse a preparing or empty manifest.
