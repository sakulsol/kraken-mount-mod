<p align="center"><img src="docs/logo.png" width="128" alt="Kraken Mount"></p>

A PS5 game mounter. It finds games on the internal SSD, the M.2 drive and USB drives, and adds
them to the home screen without installing them.

Supported formats: debug FPKG (`.pkg`) and game folders (plain, or packed with zstd or LZ4).
Other formats, such as exFAT images, are obsolete and will not be supported.

## How to

1. Download `kraken-mount.elf` from [Releases](https://github.com/quer3q/kraken-mount/releases)
   and send it to your jailbroken PS5 with the ELF loader (elfldr, port 9021) or payload-manager:
   `nc -w 3 <ps5-ip> 9021 < kraken-mount.elf`. kstuff must be loaded, and ShadowMount+ must not be
   running: Kraken Mount waits until both are true.
2. Put each game in a scan root:
   - internal SSD: `/data/homebrew`, `/data/etaHEN/games`
   - M.2 or extended drive (`/mnt/ext0`, `/mnt/ext1`): `homebrew`, `etaHEN/games` or the drive root
   - USB drive (`/mnt/usb0` to `/mnt/usb7`): `homebrew`, `etaHEN/games` or the drive root
3. Each game gets a "<name> registered" notification and its own home-screen tile.

[ampr-emu-ng](https://github.com/quer3q/ampr-emu-ng) comes bundled and is installed into
`/data/krakenmount/ampr/` at start. To run another build, put its `libSceAmpr.sprx` there; it is
kept until you delete it.

Caches live in `/data/krakenmount/caches/<TITLE_ID>`, settings in
`/data/krakenmount/config.toml` and the log in `/data/krakenmount/kraken.log`.

To build it yourself: `ps5/build.sh` (needs Docker), output `target/ps5/kraken-mount.elf`.

## Thanks

- [drakmor/ShadowMountPlus](https://github.com/drakmor/ShadowMountPlus): home-screen
  registration, the fakelib sandbox overlay and backports are ported from it.
- [ps5-payload-sdk](https://github.com/ps5-payload-dev/sdk): the PS5 toolchain.

## License

GPL-3.0-or-later (`LICENSE`). Third-party notices are in `THIRD_PARTY_NOTICES`.
