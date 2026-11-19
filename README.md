# ppextractor

Rust command-line extractor for the Japanese Game Boy Advance releases of Power Pro Kun Pocket 4 and 6. The ROM stays on your machine. The program checks its game code, revision, and exact SHA-256 before creating any output.

## Supported ROMs

| Game | GBA game code | Revision | SHA-256 |
| --- | --- | ---: | --- |
| Power Pro Kun Pocket 4 | `AP4J` | 1 | `1f1746ecc664e6c56c41197755822236c2552fe3c1ceaf8c7192ef93a05bd236` |
| Power Pro Kun Pocket 6 | `BP6J` | 0 | `72b6a3913d69710f74e9f96c7175a42901cfac486e623c2af6750bb4d487b157` |

## Usage

```sh
cargo build --release
./target/release/ppextractor supported
./target/release/ppextractor extract /path/to/pp4.gba --out ./extracted
./target/release/ppextractor extract /path/to/pp6.gba --out ./extracted --only scripts
```

`--only` accepts `all` (the default), `images`, `fonts`, `scripts`, `portraits`, or `audio`. A modified ROM or unsupported revision is rejected, and nothing is written for it.

The output root contains a `4` (PP4) or `6` (PP6) folder with this layout:

```
<out>/4|6/images/*.png          backgrounds
<out>/4|6/images/portraits/     portraits
<out>/4|6/audio/
<out>/4|6/fonts/
<out>/4|6/scripts/              script YAML
```

`--only images` extracts backgrounds only; it does not include portraits. Use `--only portraits` (or the default `all`) to fill `images/portraits`.

## Releases

Prebuilt binaries for Linux (x86_64), macOS (Intel and Apple Silicon), and Windows (x86_64) are attached to each [GitHub Release](../../releases). Each archive (`.tar.gz` or `.zip`) contains the `ppextractor` executable and this README, and `SHA256SUMS.txt` lists the checksums:

```sh
sha256sum --check --ignore-missing SHA256SUMS.txt   # macOS: shasum -a 256 -c --ignore-missing SHA256SUMS.txt
```

Releases never include ROMs or extracted assets; supply your own ROM.

To cut a release, set `version` in `Cargo.toml`, commit it, then push a matching tag. The workflow refuses tags that do not match the Cargo version, and tags with a suffix (e.g. `v0.2.0-rc.1`) are published as pre-releases:

```sh
git tag v0.1.0
git push origin v0.1.0
```

CI (`cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`) runs on every push and pull request.

This program runs entirely in Rust; Python and the source project's assets are not runtime dependencies. Keep ROM files outside Git repositories.
