# bannedcam

Bandcamp library downloader

## TUI

Browsing and downloading music from your library in TUI

```bash
bannedcam library
```

<img width="1278" height="570" alt="image" src="https://github.com/user-attachments/assets/6d9a5b9e-aea7-46e9-996a-fd53a2e62ae6" />

<img width="1278" height="570" alt="image" src="https://github.com/user-attachments/assets/41d4dafb-bd28-4c6c-8ab3-38481a3fc57f" />

## Finding your identity cookie

1. Login to your bandcamp account and open the website.
2. Open inspect element -> network tab
3. Find bandcamp.com request
4. Click on the filter headers input and type in "identity"
5. In the cookie field copy the text after ``identity=xxxxxxxxxx``

## CLI

Downloading music in cli

```bash
bannedcam download url "https://badmathhk.bandcamp.com/album/missing-narrative"
```

```
Download items from library

Usage: bannedcam download [OPTIONS] <COMMAND>

Commands:
  all   Download all items from your library
  url   Download items from urls
  help  Print this message or the help of the given subcommand(s)

Options:
      --cookie <COOKIE>      Bandcamp identity cookie (can also be set via BANDCAMP_COOKIE env vars)
  -f, --format <FORMAT>      Audio format [default: flac] [possible values: flac, mp3-v0, mp3-320, aac, ogg, alac, wav, aiff]
  -o, --output <OUTPUT>      Output directory [default: .]
      --parallel <PARALLEL>  Concurrent downloads [default: 3]
  -v, --verbose...           Increase verbosity (-v, -vv, -vvv)
      --dry-run              Show what would be downloaded without downloading
  -q, --quiet                Suppress output
      --skip-existing        Skip downloads that already exist
  -h, --help                 Print help
```

## Using with nix

```bash
nix run github:BatteredBunny/bannedcam -- library
```
