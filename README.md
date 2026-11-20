# ppextractor

Rust command-line extractor for the Japanese Game Boy Advance releases of Power Pro Kun Pocket 4 and 6. The ROM stays on your machine. The program checks its game code, revision, and exact SHA-256 before creating any output.

## Workspace

- [`crates/pp-extract`](crates/pp-extract) contains the ROM extractor and builds the `ppextractor` command.
- [`crates/pp-core`](crates/pp-core) contains the script engine and WASM bindings.

## Supported ROMs

| Game | GBA game code | Revision | SHA-256 |
| --- | --- | ---: | --- |
| Power Pro Kun Pocket 4 | `AP4J` | 1 | `1f1746ecc664e6c56c41197755822236c2552fe3c1ceaf8c7192ef93a05bd236` |
| Power Pro Kun Pocket 6 | `BP6J` | 0 | `72b6a3913d69710f74e9f96c7175a42901cfac486e623c2af6750bb4d487b157` |

## Usage

Replace `<PATH_TO_ROM>` with the path to a supported PP4 or PP6 ROM and `<PATH_TO_OUTPUT>` with the desired output directory. Keep the quotes around both paths.

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

`--only` accepts `all` (the default), `images`, `fonts`, `scripts`, `portraits`, or `audio`. A modified ROM or unsupported revision is rejected, and nothing is written for it.

### Extraction output

By default, `extract` writes all categories under `<PATH_TO_OUTPUT>/<GAME_DIR>/`. The extractor picks `4` for PP4 and `6` for PP6. Running both ROMs with the same output directory creates both game folders.

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

## Verification

The public CI runs ROM-free tests, format and Clippy checks, and a smoke test of each release binary. To require full parity tests locally, place the fixture ROMs at `<PATH_TO_ASSETS>/roms/pp4.gba` and `<PATH_TO_ASSETS>/roms/pp6.gba`, with reference outputs in `<PATH_TO_ASSETS>/extracted/pp4/` and `<PATH_TO_ASSETS>/extracted/pp6/`:

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
