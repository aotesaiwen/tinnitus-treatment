# Lyria RealTime capture

This route tests Google's experimental `models/lyria-realtime-exp` through the
official Live Music WebSocket API. It does not call Lyria 3, Lyria 3 Pro, or
Lyria 3.5, and does not enable billing or purchase a subscription.

Google's [official cookbook](https://github.com/google-gemini/cookbook/blob/main/quickstarts/Get_started_LyriaRealTime.ipynb)
describes a free preview with quota limitations. The
[Magenta announcement](https://magenta.withgoogle.com/infinite-crate-announce)
also identifies free API access for RealTime. The account's server-side access
report records a free billing tier. Availability must still be tested with a real
stream; the presence of the model in the models list is insufficient.

## Environment

Use an isolated Python 3.12 environment in `.runtime/lyria-venv` with:

```text
websocket-client==1.9.2
python-socks==3.1.1
```

`lyria_realtime_generate.py` reads `GEMINI_API_KEY` directly from the ignored
`.runtime/music-access/credentials.json`, and checks the recorded free billing
tier in `.runtime/music-access/status.json`. Credentials do not appear in command
arguments. Logging is disabled and exception/output records are scrubbed of all
credential values. The transport uses the existing SOCKS5 proxy at
`127.0.0.1:1080`, with remote DNS resolution and normal TLS verification.

```sh
.runtime/lyria-venv/bin/python research/listening/lyria_realtime_generate.py
```

## Sampling protocol

Two fixed initial seeds, `210731` and `210732`, request at least 40 seconds each with the
same restrained acoustic solo-piano prompt. Configuration uses 64 BPM, D major /
B minor scale, density 0.2, brightness 0.3, guidance 5.0, temperature 0.9, top-k
40, quality mode, and drums muted. Piano bass notes remain permitted. The server
may not follow the prompt; the resulting instrumentation and sound quality must
be evaluated by listening, not assumed from these settings.

The script saves every received PCM chunk unchanged, an equivalent WAV container,
and a sanitized JSON record under
`artifacts/audio/blind-20261021/lyria-realtime/`. The stream stops once it reaches
the duration threshold, so the final chunk can make it slightly longer. Audio
is stereo signed 16-bit PCM; sample rate comes from the MIME type, or the
documented 48 kHz default. No filtering, loudness matching, fades, denoising, or
cherry-picking occur in this capture step. The common blind-set builder performs
its own disclosed duration/loudness preparation later.

Each record contains the precise prompt/configuration, chunk timing and source
metadata, raw audio hashes, stereo/duration/level checks, and any failure stage.
If the first request fails, the second is skipped. Existing outputs are never
overwritten. A local setup failure before network connection is retained in
`setup-diagnostics/missing-socks-adapter.json`; it made no API call.

Reference: [Live Music API schema](https://ai.google.dev/api/live_music),
[RealTime guide](https://ai.google.dev/gemini-api/docs/realtime-music-generation).

## Captured results, 2026-10-21

Both sessions succeeded on the existing free account. Each returned 20 unchanged
2-second PCM chunks: 40.0 seconds, 48 kHz, stereo. The server echoed the requested
prompt/configuration and incremented its seed by one per chunk (initial seeds
210731 and 210732). The complete per-chunk metadata is retained; no 2-second PCM
chunk is identical between the two recordings. There were no server warnings or
filtered prompts. Independent WAV/PCM checks, hashes, peaks and short-window
levels are in the generated `validation.json`. Instrument compliance and musical
quality remain listening questions; a successful API call does not establish
that the model produced clean solo piano.
