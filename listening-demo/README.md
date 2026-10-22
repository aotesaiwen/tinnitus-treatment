# Listening Room

A small static blind-listening demo. Each browser gets a shuffled order and
anonymous sample letters. The listener records sound clarity, musicality, desire
to keep listening, optional artifacts, and notes before revealing the sources.
Ratings stay in that browser's local storage and can be exported as JSON.

The standalone public demo is hosted at
[aotesaiwen.github.io/tinnitus-treatment](https://aotesaiwen.github.io/tinnitus-treatment/).
Visitors do not need ChatGPT or GitHub accounts. HTML, scripts, styles, and audio
are served together; no API keys, model downloads, external fonts, analytics, or
backend service are needed to listen.

The [repertoire page](https://aotesaiwen.github.io/tinnitus-treatment/repertoire/)
adds complete, named pieces played through the same piano renderer. Chopin's
Nocturne Op.9 No.2 is available as a full 204.4-second rendition from Mutopia's
MIDI. The second selection, the user's exact MuseScore arrangement of *Heolstor,
the Night Lord*, is clearly marked pending because the score page returned a
browser verification challenge and no score file was obtained.

The former ChatGPT Site remains separate. `.openai/hosting.json` records that
legacy deployment and is not included in the public site's publishing branch.
Browser-local scores belong to each origin, so scores on the old hostname do not
automatically appear on the new hostname.

## Publish the standalone site

GitHub Pages serves `/` from the `codex/listening-pages` branch. That branch
contains only the committed contents of `listening-demo/dist`; `.nojekyll` keeps
the files unchanged. After committing and verifying a change, publish from the
repository root:

```sh
listening_commit=$(git subtree split --prefix=listening-demo/dist)
git push origin "$listening_commit:refs/heads/codex/listening-pages"
```

The push must be a fast-forward; do not force-push the deployment branch. Wait
for the Pages build to complete, then check the public homepage, both manifests,
and audio playback without authentication. Test direct access without a proxy
when assessing local network reachability; one successful network test does not
guarantee access from every ISP.

## Preview

From the repository root, with the existing app dependencies installed:

```sh
node app/node_modules/vite/bin/vite.js preview --outDir listening-demo/dist --host 127.0.0.1 --port 8765
```

Use a static server that supports HTTP byte-range requests. A plain Python
`http.server` can play these FLAC files but prevents seeking in Chromium.
Open the printed URL; opening `index.html` directly does not support JSON fetches.

The second round contains twelve short listening excerpts. The first round's six
excerpts and existing browser-local scores remain accessible at `round-1/`, using
their unchanged sample IDs and storage version. Full raw audio and
model weights stay outside Git. To rebuild the excerpts locally, follow
[`research/listening/README.md`](../research/listening/README.md).

## Session behavior

- Starting a round shuffles actual excerpts; reloading preserves that round's
  labels, scores, notes, and position in the sample list.
- The round links switch between separately saved datasets. Resetting or revealing
  one does not change the other.
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
export, reset, narrow layouts, and preservation of first-round scores when
switching datasets. It saves screenshots under `.runtime/previews`.

For the complete-piece page, run
`node research/listening/check_repertoire.cjs http://127.0.0.1:8765/repertoire/`.
Its source, license, renderer, original MIDI hash, output hash, and gain-only
processing are recorded in `research/listening/repertoire-record.json`.

`dist/` is the entire deployable app: HTML, CSS, JavaScript, manifest, reveal map,
and lossless audio. There is no build step. Publish an exact committed snapshot
including the audio; do not reuse a preparing or empty manifest.
