# ppextractor

Rust command-line extractor for Japanese Power Pro Kun Pocket ROMs. The ROM stays on your machine. GBA extraction checks the game code, revision, and exact SHA-256 before creating output.

## Workspace

- [`crates/pp-extract`](crates/pp-extract) contains the ROM extractor and builds the `ppextractor` command.
- [`crates/pp-core`](crates/pp-core) contains the script engine and WASM bindings.
- [`crates/pp-sound`](crates/pp-sound) replays PP6's sound driver (Konami sequencer, GB PSG, m4a PCM mixer) natively or as WASM. See [PP6 sound](#pp6-sound).

## Supported ROMs

The extractor currently supports the eleven GBA ROM profiles shown by `ppextractor supported`. The GBA title Pawapoke Dash, the Game Boy and Game Boy Color originals, Power Pro Kun Pocket 8–14 for Nintendo DS, and Power Pro Kun Pocket R for Nintendo Switch still need ROM fixtures and title-specific asset decoders. The DS cartridges have an initial raw-file extractor. See the [KONAMI series soundtrack catalog](https://www.konami.com/pawa/pawapoke_r/topics/sound) and the [Power Pro Kun Pocket R release announcement](https://www.konami.com/games/corporate/ja/news/topics/20210805/). Run `ppextractor supported --catalog` for a TSV of all registered or catalogued fingerprints and extraction entry points. Use `ppextractor identify ROM [ROM ...]` to hash local files and look up their catalog entries independently of their filenames. See the [cartridge revision inventory](docs/rom-revisions.md) for catalogued hashes and remaining fixtures.

| Profile | GBA game code | Revision | SHA-256 |
| --- | --- | ---: | --- |
| `pp4` | `AP4J` | 1 | `1f1746ecc664e6c56c41197755822236c2552fe3c1ceaf8c7192ef93a05bd236` |
| `pp6_v0_clean` | `BP6J` | 0 | `65f994ecf0353ebe1d90f21fc783b3a4ae7994503f986005c0052eb87c36003e` |
| `pp6` | `BP6J` | 0 | `72b6a3913d69710f74e9f96c7175a42901cfac486e623c2af6750bb4d487b157` |
| `pp1_2_v0` | `B8PJ` | 0 | `6e5830a7c385546205d3e50343a028aba54ff0d315f41d0604fd021e59d4497d` |
| `pp1_2_v0_p` | `B8PJ` | 0 | `7808ccf2ac1ec89d6252291464313b3b98218e3b3f363d6d41f07a2f31e10da7` |
| `pp3_v0` | `AP3J` | 0 | `3a75c1ad9fc27375bac764b820b542e9444a3ba71ec10d99b6ca89f0517b2cbc` |
| `pp3_v1` | `AP3J` | 1 | `ca6fafb3746782199beaedd25a2fdeee7bb44049bf35f80bb8e0115e3eae17d7` |
| `pp4_v0` | `AP4J` | 0 | `97f00aac3b27e1507936c01f22e6ac4ab3c01cb9139978549487f26c77789a1b` |
| `pp5_v0` | `A5PJ` | 0 | `e70347f919664dd96af44e35062b5712daeed38e91e377d464777c6d4d436ddd` |
| `pp5_v0_p` | `A5PJ` | 0 | `2800f7f7f8762b6169c8273249a013d90006eeedb4d36d96ddc4ca70e3040294` |
| `pp7_v0` | `BP7J` | 0 | `54fe895a178b51c6b93e15c8410c32df1aaa04c2c6bdb717695b33ec692f50bf` |

For Switch cartridge images, `ppextractor xci-index <ROM.xci> --out <PATH_TO_OUTPUT>`
verifies the root HFS0 header and records partition names, file sizes, and absolute
file offsets in `xci-index.json`. Add `--copy-partition secure` to stream the validated partition files to `partition-files/` as raw bytes. The catalog-sized Power Pro Kun Pocket R image
is matched by its full SHA-1. Use `ppextractor romfs-files <ROMFS_IMAGE> --out <PATH_TO_OUTPUT>` on an already
decrypted Switch RomFS image to extract raw files under numeric paths and record
their original directory paths in `romfs-index.json`. Add `--decode-res00` to try decoding sibling `RES00.RDI` and `RES00.RDB` files automatically and record pair results in `res00-pairs.json`. A unique sibling `RES11.RDB` also supplies raw patch ranges when the index validates against it. Game asset decoding and
real-cartridge validation are pending.

Use `ppextractor xci-files ROM.xci --program NAME.nca --keyset KEYS --out OUTPUT`
to extract a named NCA from the secure partition and immediately process its
ExeFS and RomFS. Use the NCA's original name shown by `xci-index`. The command
checks its HFS0 hash when present and removes the temporary NCA copy. It
supports the same decoding flags as `nca-files`. A synthetic XCI with an
AES-XTS-encrypted NCA header and AES-CTR-encrypted ExeFS and RomFS sections
was tested with hactool revision `1d64a83` through RES00 unpacking and CHK
string export. All 15 outputs matched the unencrypted-section baseline.
Retail Pocket R extraction still requires a fixture.

For a program NCA copied from the secure partition, run
`ppextractor nca-files PROGRAM.nca --keyset KEYS --out OUTPUT` with local
`hactool` on PATH, or set `--hactool` explicitly. The keyset is supplied to
hactool locally. The command passes the decrypted sections to the ExeFS and
RomFS extractors and removes the temporary section images. Use
`--decode-main` or `--decode-res00` to enable the corresponding asset decoding.
For an NSP package, list its contents with `ppextractor nsp-index PACKAGE.nsp
--out OUTPUT` and check `nsp-index.json` for the NCA filenames. Use
`ppextractor nca-files PACKAGE.nsp --program NAME.nca
--keyset KEYS --out OUTPUT` to select its program NCA. For an update that
depends on the base game's RomFS, add `--base-nca BASE.nca`.
The `--content` alias can select a data NCA in a DLC package. A synthetic
encrypted data-only NSP yielded all eight expected RomFS and RES00 outputs
with hactool revision `1d64a83`. Pocket Dash for Switch still needs a retail
DLC fixture to confirm its resource layout.
With hactool revision `1d64a83`, an encrypted synthetic BKTR relocation
reusing the base RomFS produced the same 15 files as extracting the base
directly. A second synthetic patch used 12 base and patch ranges to change a
decoded CHK string. Its 15 outputs matched a full image of the updated data
byte for byte.
For an NCA encrypted with a title key, hactool reads `title.keys` from
`~/.switch/` or its XDG configuration directory (for example,
`$XDG_CONFIG_HOME/switch/title.keys`). The `--keyset` argument still points to
the separate hactool keyset. With hactool revision `1d64a83`, a synthetic NCA
using title-rights encryption produced the same 15 files as the equivalent
test NCA using key-area encryption.

Use `ppextractor exefs-files EXEFS --out OUTPUT --decode-main` when the
decrypted ExeFS is still a PFS0 image. The original entry names are recorded
in `exefs-index.json`, and the optional flag exports the main executable's
segments under `main/`.

Use `ppextractor nso-segments MAIN --out OUTPUT` on the `main` file from an
already decrypted Switch ExeFS. It extracts `text.bin`, `rodata.bin`, and
`data.bin`, expands LZ4 segments, and verifies hashes when the NSO header
requires them. `nso.json` records each segment's load offset.

The experimental `res00-files --index <RDI> --data <RDB> --out <OUTPUT>`
command reads the published RES00 layout, indexes stored resources, and
copies their raw RDB bytes. For a patch-bearing RDI2 index, it extracts the base entries and records the patch table in `res00-patch-index.json`. Pass `--patch-data <RES11.RDB>` to copy its indexed ranges into `patch-chunks/`. Applying the patch records to base assets remains open. It accepts plaintext RDI2 and indexes encoded with the cipher used by the published unpacker. Pass `--decode-chunks` to also try decoding named chunks and inflating zlib or raw DEFLATE payloads. Compatible UTF-8 `STRING` tables in decoded `CHK ` payloads export under `strings/`. Compatible `NX  SUR` textures also export their original GPU-layout bytes and descriptors under `textures/`, with PNG previews for supported BC1, BC3, BC4, BC5, and BC7 surfaces. Compatible `UNCDFONT` chunks export under `fonts/` with code points, metrics, and verified bitmap ranges. These layouts are documented for the related Powerful Pro Yakyu 2024–2025 release and remain unverified on Pocket R. Raw chunks retain every stored byte, while decoding operates on complete 32-bit words. Per-chunk decode failures are recorded in `res00-index.json`. Entries that share an identical RDB byte range reuse its raw output path. This step
has synthetic coverage and awaits validation on Power Pro Kun Pocket R.
See the [original Pocket R format investigation](https://gbatemp.net/threads/pocket-r-help-how-to-decompress-the-zlib-deflate-the-rdi-and-rdb-file.611799/)
and the [related RES00 unpacker](https://github.com/masagrator/NXGameScripts/blob/main/Powerful%20Pro%20Yakyu%20Baseball%202024-2025/UNPACK_RES00.py).

For the catalogued GBA Power Pocket Dash and Power Pro Kun Pocket 5 Rev 1 dumps,
`ppextractor gba-scan <ROM.gba> --out <PATH_TO_OUTPUT>` verifies the SHA-1 and
runs an experimental scan using the existing GBA background record structure.
`extract` runs this scan for these two exact hashes with `--only all` or
`--only images`. With `--only samples` or `--only all`, it also searches for
a contiguous PCM bank confirmed by an independent relative-offset table and
exports it when the match is unique. The detector reproduces the known bank
offsets and counts on all eleven local GBA profiles. PP5 Rev 1 additionally
supports `--only audio` when the PP5 playback tables match and all BGM,
effect, and percussion tracks pass validation. With `--only all`, native
audio is preferred when available and PCM sample extraction is the fallback. With `--only fonts` or
`--only all`, it also searches for a unique dialogue-font pointer table
matching a known GBA layout and exports its glyphs with inherited Unicode
assignments. With `--only portraits` or `--only all`, it validates and
exports compatible 56×56 OBJ bust banks. The two local PP5 Rev 0 variants
both reproduce their known 217-image bank. With `--only scripts` or
`--only all`, the whole-ROM scan exports raw event bodies whose descriptors
have an aligned pointer from outside the event. These are candidates; the
scanner can miss events without aligned references. For PP5 Rev 1, a match
against the relocation-independent PP5 Rev 0 handlers and entry routine also
enables provisional TSV dialogue listings. Dash and PP5 Rev 1 still require
retail fixtures to verify their layouts and mappings.
It exports candidate PNGs, a catalog, and scan counts. These layouts still need
verification on the corresponding cartridges.

To split a catalogued GB/GBC cartridge dump into its original ROM banks, run
`ppextractor gb-banks <ROM.gbc> --out <PATH_TO_OUTPUT>`. The command verifies
the catalog SHA-1 for Power Pro Kun Pocket Rev 0 and Rev 1 and Power Pro Kun
Pocket 2 Rev 0. It exports 16 KiB banks, source offsets, and header metadata.
Use `ppextractor gb-tiles <ROM.gbc> --out <PATH_TO_OUTPUT> --offset 0x4000 --count 32`
to preview a ROM range as 8×8 grayscale tiles. The offset is caller-selected;
this does not identify which bytes the game uses as graphics.
Use `ppextractor gb-dialogue INPUT --offset 0xSTART --length LENGTH --out dialogue.tsv`
to inspect a known Success dialogue range in a PP1 ROM or extracted bank.
Omit `--length` to extract from a known event start through its parsed `FFFF`
terminator within the same 16 KiB ROM bank. This mode reports an error if an
unrecognized instruction or the bank boundary prevents determining the length.
Both GB dialogue commands select PP2's character table for a catalogued PP2 ROM or a bank under its extraction manifest. Use `--pp2` for an independent PP2 file or byte range. The output decodes
explicit choice lists and preserves operands. It retains the remaining bytes
at unrecognized commands or choice modes.
Use `ppextractor gb-find-dialogue INPUT --text '会話の一部'` to find candidate
file offsets and byte lengths for a known phrase, adding `--pp2` for PP2.
Pass those offsets and lengths to `gb-dialogue` with the same input and title
option. Binary data outside scripts can also match, so a hit is a candidate.
Automatic event discovery and validation on real GB/GBC cartridges remain open.

To extract the raw Nintendo DS cartridge files, run
`ppextractor nds-files <ROM.nds> --out <PATH_TO_OUTPUT>`. The command
recognizes the catalogued DS game codes and writes numeric FAT IDs and their ROM
ranges under a directory keyed by title, revision byte, and SHA-256. The `rom.json` manifest records the hashes and catalog revision when the dump matches a known DS image. The index also records original NitroFS paths when the cartridge has a file name table. Standard NARC archives are unpacked under `narc/<hex FAT ID>/`, with offsets relative to the archive data block. Nested NARC and SDAT members recurse within the depth and extraction budgets. The top-level extraction preserves each original archive. Raw ARM9/ARM7 executables and any DSi extensions export under `executables/`. The manifest records file offsets, load addresses, and executable extraction errors. When the ARM9 module settings identify a valid BLZ region, `executables/arm9-decompressed.bin` is also written and indexed. ARM9 and ARM7 overlays export under `overlays/` with FAT references, RAM metadata, and BLZ-expanded binaries when applicable. LZ10, LZ11, Nintendo RLE, and 4-/8-bit Nintendo Huffman files identified by extension, a known decoded signature, or a complete stream with at most alignment padding also export under `decompressed/`. Compressed NARC and SDAT files feed the same archive extraction as their uncompressed counterparts. Nitro NCGR and NCLR resources export grayscale tile previews and palette swatches under `tiles/` and `palettes/`. Text-mode, affine, and extended affine NSCR maps paired with NCGR and NCLR files of the same path stem also export color PNGs under `backgrounds/`, including mapped palettes and backgrounds made of multiple screen panels. NANR animation banks export frame durations, cell references, and transforms under `animations/`. Matching cell resources also export transformed frame PNGs with frame durations and coordinates. NCER cell banks export object positions, dimensions, tile indices, and raw attributes under `cells/`. Matching NCGR and NCLR filenames also produce PNGs and origin coordinates under `sprites/` for non-affine cells, including 1D, 2D, and per-cell VRAM transfers. NFTR fonts export bitmap sheets, character codes, cell metrics, and per-glyph CWDH spacing under `fonts/`. Glyphs are shown in their stored orientation. BMG message resources export text, entry attributes and raw control codes under `messages/`. BMG flow and script tables export optional `.flow.json` files with original instruction slots and entry indices. DS and DSi banners export their static icon, localized titles, and original bytes under `banner/`. DSi animations also export PNG frames and an `animation.tsv` timeline with durations in VBlanks. Standalone SWAV and SWAR entries, including entries inside NARC archives, export 8-bit PCM, 16-bit PCM, and ADPCM previews under `wav/`. The WAV previews keep the encoded sample rate. Looping SWAV, SWAR, and STRM entries also export a `.loop.json` file with the original loop in decoded sample frames. STRM streams with 8-bit PCM, 16-bit PCM, or ADPCM also export interleaved-channel WAVs. SDAT archives also export their raw entries and supported WAV previews under `sdat/<hex file ID>/`. SDAT index offsets are relative to the original archive. SSEQ files using supported commands export MIDI previews at 48 ticks per quarter note, including multiple tracks, subroutines, jumps, and counted loops. A top-level repeating jump or indefinite loop exports its intro and first pass with MIDI loop markers. SSAR archives also export individual MIDI previews and a sequence index with entry offsets and bank IDs. Sequences that use unsupported commands retain their raw data. The SYMB block, when present, supplies `sound-symbols.tsv` with names and SSAR subsequence names. When present, the INFO block supplies `sound-info.tsv` with logical sound IDs, raw file IDs, sequence bank IDs, and bank wave archive IDs. SBNK tables under `banks/` map programs and key ranges to wave slots or PSG settings and preserve envelope values. This stage
uses the Nintendo DS FAT and has been verified with a synthetic cartridge image.
Game-specific decoding and verification against real DS cartridges remain open.
See [ndstool's Nintendo DS header and extraction implementation](https://github.com/devkitPro/ndstool/tree/master/source).

To render a background with differently named resources, use
`ppextractor nds-background --screen MAP --tiles GRAPHICS --palette COLORS --out image.png`.

Use `ppextractor nds-find-dialogue INPUT --text '会話の一部' --game pp10`
to find candidate byte offsets and lengths for a known line. Search the ROM or
an extracted, decompressed file. Use the game's glyph forms (e.g. full-width
Latin letters). Matches can also occur in non-dialogue data. Pass the output
directory from `nds-files` to search its `.bin` files recursively, including
decompressed files and archive members. Directory search adds a relative
`path` column, and offsets are relative to each matched file.

Use `ppextractor nds-dialogue INPUT --offset 0xSTART --length LENGTH --out dialogue.tsv`
to inspect a known Success dialogue range in a ROM or extracted file. Offsets in
the TSV are relative to the input file. It decodes common kana, kanji,
symbols and control codes, retains raw glyph codes and operands, and reports
the remaining bytes at the first unrecognized control. Locating script ranges and mapping
title-specific glyphs still require per-title analysis. Add `--game pp10` (or
the applicable DS title) to enable documented title-specific operands,
including short conditions in the two Koushien titles and ruby in PP14.
Without `--game`, both dialogue commands infer the title from a `.nds`
header or the nearest `rom.json` above an extracted file or directory. The
address data following an `FF` marker is preserved as raw bytes.
The inputs may be raw or supported compressed Nitro files. Automatic pairing recognizes an additional compression suffix such as `.ncgr.lz` and skips ambiguous duplicate matches.

## Usage

Replace `<PATH_TO_ROM>` with the path to a supported ROM and `<PATH_TO_OUTPUT>` with the desired output directory. Keep the quotes around both paths.

### Build from source

From the repository root, with Rust and Cargo installed:

```sh
cargo build --release --locked -p pp-extract
./target/release/ppextractor supported
./target/release/ppextractor extract "<PATH_TO_ROM>" --out "<PATH_TO_OUTPUT>"
```

On Windows, run `.\target\release\ppextractor.exe` from PowerShell.

### Use a release binary

Download the archive for your platform from [GitHub Releases](https://github.com/tkgstrator/pp-core/releases). Extract it, then open a terminal in the extracted folder. Release archives are built for Linux x86-64, macOS Apple Silicon and Intel, and Windows x86-64.

Linux and macOS:

```sh
./ppextractor supported
./ppextractor extract "<PATH_TO_ROM>" --out "<PATH_TO_OUTPUT>"
./ppextractor extract "<PATH_TO_ROM>" --out "<PATH_TO_OUTPUT>" --only scripts
```

Windows (PowerShell):

```powershell
.\ppextractor.exe supported
.\ppextractor.exe extract "<PATH_TO_ROM>" --out "<PATH_TO_OUTPUT>"
.\ppextractor.exe extract "<PATH_TO_ROM>" --out "<PATH_TO_OUTPUT>" --only scripts
```

For `.gb` and `.gbc` files, `extract` runs the catalog-verified GB/GBC bank extractor. For `.nds` files, it runs the Nintendo DS file and Nitro asset extractor. A known GB/GBC SHA-1 or a recognized DS header and file table also works without these extensions. These platforms use the default `--only all` and retain the same output layout as `gb-banks` and `nds-files`.

For GBA, `--only` accepts `all` (the default), `images`, `fonts`, `scripts`, `portraits`, `audio`, or `samples`. GBA hashes outside the extraction and scan catalogs, and categories unavailable for a given ROM, are rejected before creating output.

### Extraction output

By default, `extract` writes the categories available for the verified ROM under `<PATH_TO_OUTPUT>/<GAME_DIR>/`. PP4 revision 1 and the previously registered PP6 revision 0 profile retain output directories `4` and `6`. The catalog-matching PP6 revision 0 image supports the same five categories and writes to `pp6_v0_clean`.

PP4 revision 0 supports all five categories under `pp4_v0`. PP1・2 revision 0 (both registered hashes), PP3 revisions 0 and 1, PP5 revision 0 (both registered hashes), and PP7 revision 0 currently support background images. PP1・2, PP3, PP5, and PP7 also support dialogue fonts with inherited Unicode assignments documented in `glyphs.json`. These ROMs write to their profile IDs (for example, `pp4_v0/images`) so revisions and variants can share one output root. PP3 and PP5 export 56×56 portrait busts. PP1・2 exports dialogue portraits and records the scenario-dependent bank selection and left-side remapping in the portrait manifest. PP7 exports complete dialogue portraits with their runtime portrait IDs. Both preserve original sprite bounds. PP1・2, PP3, PP5, and PP7 export byte-exact candidate event bodies and aligned pointer references under `scripts/native`. Pointer hits are structural evidence. PP1・2, PP3, PP5, and PP7 also write provisional VM listings using the corresponding PP4 command layout and inherited glyph assignments. Candidates containing unchecked extended opcodes or portrait sentinels remain in raw form. The listings preserve the original bytes and include provisional call and jump targets in `scripts/native/edges.tsv`. Structurally valid targets reached through those instructions are exported as well, with provenance in `scripts/native/linked.tsv`; title-specific command behavior and routes still require verification. Direct Thumb calls that load an event descriptor and call the VM entry routine appear in `scripts/native/entrypoints.tsv`. The `mode` column records the explicit launch mode or mode 0 when the call uses the default wrapper. For indexed calls, `table_index` identifies the slot in a verified ROM table. For calls that use a second table, `subtable_index` identifies the slot in that table. Those rows describe potential targets across game states. The `sound-requests.tsv` index lists encoded sound IDs by event and instruction offset. The sound driver can remap these IDs at runtime. The `conditions.tsv` index records true and false instruction offsets within each event. A missing false target is represented by an empty field. The provisional `switches.tsv` index records each selector slot and the candidate case and exit offsets found using the PP4 marker layout. The static graph rooted at those descriptors is listed in `reachable.tsv`, with shortest path lengths. These paths do not evaluate runtime conditions. All registered ROMs can export indexed PCM samples with `--only samples`. This writes WAV files, MIDI previews, raw records and `audio/samples-manifest.json`. PP1・2, PP3, PP5, and PP7 also export native BGM, sound-effect and percussion sequences with `--only audio`. PP1・2, PP3, and PP5 also generate sequence MIDI, approximate reference WAVs, and a companion SoundFont. PP7 generates MIDI and approximate WAV previews for supported sequences, plus a companion SoundFont. Its manifest records static and timed sound calls. PCM rate lookup uses the same seven-bit integer pitch as the PP7 track driver. Sound calls are mixed into reference previews at the decoded update, within the calling sequence’s duration cap. PP7 preserves its packed descriptor fields and original track bytes, and records linear token counts and terminator offsets. Run `supported` to see the exact hashes and categories.

```text
<PATH_TO_OUTPUT>/
└── <GAME_DIR>/
    ├── images/
    │   ├── catalog.tsv
    │   ├── <BACKGROUND_ID>.png
    │   └── portraits/
    │       ├── manifest.json
    │       ├── actors/              PP6 only
    │       ├── banks/
    │       └── index/
    ├── fonts/
    │   ├── dialogue.otf
    │   ├── dialogue.ttf
    │   ├── glyphs.json
    │   └── glyphs.png
    ├── scripts/
    │   ├── index.tsv
    │   ├── state-links.tsv
    │   ├── scenes/
    │   │   └── <SCENE_ID>.yaml
    │   └── native/
    └── audio/
        ├── manifest.json
        ├── rom.sf2
        ├── bgm/
        ├── se/
        ├── samples/
        └── native/
            ├── bgm/
            ├── se/
            ├── samples/
            └── percussion/
```

`--only` creates just the requested category. For example, `--only scripts` writes to `<PATH_TO_OUTPUT>/<GAME_DIR>/scripts/`. Use `--only portraits` for portrait files under `images/portraits/` and `--only images` for backgrounds and their catalog. Files inside `scripts/native/` vary by game.

## Scripts

`scripts/scenes/*.yaml` contains the version 2 scene format. Each scene has an opaque `scene_id` and an ordered `commands` list. Commands use shared types such as `TEXT`, `EFFECT`, `CHOICE`, `WAIT`, `IF`, `SET`, `CALL`, and `JUMP`, with named fields. Scene and command IDs are identifiers, not ROM addresses. The primary YAML does not require `offset`, `bytes_hex`, or `args`.

`scripts/index.tsv` links scene IDs to their YAML files. The original lossless ROM records, including raw bytes and offsets, are kept separately in `scripts/native/`; `scripts/state-links.tsv` records the source of opaque state IDs. `SE`, `BGM`, and `RETURN` are reserved command types until their dialogue opcode mappings are verified. `UNRESOLVED` marks instructions whose effects are still unknown, so these scenes are not yet a complete playable reconstruction. See [the script format](docs/script-format.md) for the command fields and limits.

See the [extended PP script format](docs/pp-core/scripts/format.md) for the YAML specification for `pp-core` playback.

## PP6 sound

`pp-sound` is a port of PP6's own sound driver, documented in [docs/pp6-sound-driver.md](docs/pp6-sound-driver.md). It reads only the ROM ranges the driver needs, packed into a blob (about 1.1 MB, mostly PCM samples):

```sh
cargo run --release -p pp-extract --example pp6_sound_blob -- <PATH_TO_PP6_ROM> pp6-sound.bin
cargo build --release -p pp-sound --lib --target wasm32-unknown-unknown   # target/.../pp_sound.wasm
cargo run --release -p pp-sound --example render -- pp6-sound.bin 68.wav bgm 68 60
```

The `render` example writes 32,768 Hz WAV in mGBA's output units. `DATA` may also be a whole ROM image, including one patched so that only one track of a song plays. `PP_TIMING=timer,counter,vblank,latency` lines the clocks up with an mGBA recording (`58440,4081,20000,280896` for the recordings in the driver notes).

The WASM exports a C ABI:
- `pp_alloc` and `pp_init(ptr, len)` load the blob.
- `pp_player_new()` creates a player handle.
- `pp_play_bgm(h, id)`, `pp_play_se(h, id)`, `pp_stop(h)` and `pp_set_volume(h, v)` control playback.
- `pp_render(h, frames, rate)` returns a pointer to interleaved stereo `f32` at any output rate, after the console's DC high-pass.

## Verification

The public CI runs ROM-free tests, format and Clippy checks, and a smoke test of each release binary. To require full parity tests locally, use fixture ROMs named `pp4_v1.gba` and `pp6_v0.gba` under `<PATH_TO_ASSETS>/roms/` and reference outputs under `<PATH_TO_ASSETS>/extracted/4/` and `<PATH_TO_ASSETS>/extracted/6/`. The legacy ROM names `pp4.gba` and `pp6.gba` and Python reference folders `extracted/pp4/` and `extracted/pp6/` also work.

```sh
PPEXTRACTOR_ASSETS_ROOT="<PATH_TO_ASSETS>" PPEXTRACTOR_REQUIRE_ROM_PARITY=1 cargo test --all-targets --locked
```

This mode fails if a ROM or reference output is missing. ROMs and reference outputs are not committed or packaged.

## Releases

Prebuilt binaries for Linux (x86_64), macOS (Intel and Apple Silicon), and Windows (x86_64) are attached to each [GitHub Release](https://github.com/tkgstrator/pp-core/releases). Each archive (`.tar.gz` or `.zip`) contains the `ppextractor` executable and this README, and `SHA256SUMS.txt` lists the checksums:

```sh
sha256sum --check --ignore-missing SHA256SUMS.txt   # macOS: shasum -a 256 -c --ignore-missing SHA256SUMS.txt
```

Releases never include ROMs or extracted assets; supply your own ROM.

To cut a release, set `version` in `crates/pp-extract/Cargo.toml`, commit it, then push a matching tag. The workflow refuses tags that do not match the Cargo version, and tags with a suffix (e.g. `v0.2.0-rc.1`) are published as pre-releases:

```sh
git tag "<RELEASE_TAG>"
git push origin "<RELEASE_TAG>"
```

CI (`cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`) runs on every push and pull request.

This program runs entirely in Rust; Python and the source project's assets are not runtime dependencies. Keep ROM files outside Git repositories.

## Extended PP player

Build the WASM package, then serve the repository root:

```sh
rustup target add wasm32-unknown-unknown
wasm-pack build crates/pp-core --target web --release --out-dir ../../web/pkg -- --features wasm --locked
python3 -m http.server "<PORT>" --bind "<BIND_IP>"
```

Open `http://<SERVER_IP>:<PORT>/web/` to play the documented scenario. On a remote host reached over Tailscale, use the host’s Tailscale IP for `<BIND_IP>` and `<SERVER_IP>`. To load your own script YAML, also provide its [required scenario configuration](web/scenario.yaml). You can optionally upload image or audio files. Use the asset ID as each filename stem (for example, `<BACKGROUND_ID>.png` or `<SE_ID>.wav`). Set `start_scene` in the configuration to a scene in the uploaded script. The built-in demo uses placeholder portraits and synthesized audio. The browser implements the `shake` effect.
