# Check media and GitHub text

Install `ffmpeg` and `whisper-cli` (whisper.cpp). The first media run needs
`--download-model`, or an interactive yes, to fetch the pinned ggml weights.

```bash
pangram video ./lecture.mp4 --max-billable-units 80 --download-model
pangram audio ./interview.wav --model large-v3-turbo-q5_0 --max-billable-units 40
pangram youtube 'https://www.youtube.com/watch?v=...' --max-billable-units 80
```

`youtube` runs your own `yt-dlp`. Pangram CLI does not install it, and using
it is your YouTube-access decision.

GitHub shortcuts read `GH_TOKEN` or `GITHUB_TOKEN`:

```bash
pangram pr owner/repo#123 --max-billable-units 5
pangram issue https://github.com/owner/repo/issues/9 --comments --max-billable-units 10
pangram comments owner/repo#123 --max-billable-units 10
```

Review comments send only their `body`, never the diff hunk. Output uses
`origin: transcript` or `origin: github`.
