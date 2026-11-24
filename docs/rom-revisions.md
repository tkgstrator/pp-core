# Cartridge revision inventory

The tables record catalogued Japanese releases and their full-image hashes. A local filename means an exact SHA-1 match with the ROM fixtures used to develop the extractor. The profile registry uses SHA-256 for extraction. A dash in the local column means the catalogued image still needs a fixture. DS secure-area encryption and Switch updates can produce other images. This is a snapshot of the linked catalogs.

## GBC

[Catalog snapshot](https://raw.githubusercontent.com/libretro/libretro-database/90601bff67dba7057381292f17e08ed2cecbd6dc/metadat/no-intro/Nintendo%20-%20Game%20Boy%20Color.dat)

| Release | Code | Bytes | SHA-1 | Matching local file |
| --- | --- | ---: | --- | --- |
| Power Pro Kun Pocket (Japan) (Rev 1) (SGB Enhanced) (GB Compatible) | — | 2097152 | `ae145f2de0d53c19b973c71baaf3f2cca2ca395a` | — |
| Power Pro Kun Pocket (Japan) (SGB Enhanced) (GB Compatible) | — | 2097152 | `f796f9cb259646555f0d429b9337ac6b423755a3` | — |
| Power Pro Kun Pocket 2 (Japan) (SGB Enhanced) (GB Compatible) | — | 2097152 | `5c0ba404cf30296dceeb427a54e3644415c9e8db` | — |

## GBA

[Catalog snapshot](https://raw.githubusercontent.com/libretro/libretro-database/90601bff67dba7057381292f17e08ed2cecbd6dc/metadat/no-intro/Nintendo%20-%20Game%20Boy%20Advance.dat)

| Release | Code | Bytes | SHA-1 | Matching local file |
| --- | --- | ---: | --- | --- |
| Power Poke Dash (Japan) | BBQJ | 8388608 | `598db3dd414ec3acb8bea47192761816e401df15` | — |
| Power Pro Kun Pocket 1, 2 (Japan) | B8PJ | 8388608 | `844512db89fca8476c0c585d8699b3446495eabc` | pp1_2_v0.gba |
| Power Pro Kun Pocket 3 (Japan) | AP3J | 8388608 | `c6d7317d945f96034699501ac33b1966c5d2c3de` | pp3_v0.gba |
| Power Pro Kun Pocket 3 (Japan) (Rev 1) | AP3J | 8388608 | `ab501ba0c07e7fecbfb15e837b94a08252203c12` | pp3_v1.gba |
| Power Pro Kun Pocket 4 (Japan) | AP4J | 8388608 | `6ab43fd731c1d8b6388d6719b8bcbce698da51de` | pp4_v0.gba |
| Power Pro Kun Pocket 4 (Japan) (Rev 1) | AP4J | 8388608 | `af8006b7ef649ca35f21d53137f48dc5e6209f5e` | pp4_v1.gba |
| Power Pro Kun Pocket 5 (Japan) | A5PJ | 8388608 | `15bc24f0f47e0317c849c1c33a36357a43def74c` | pp5_v0.gba |
| Power Pro Kun Pocket 5 (Japan) (Rev 1) | A5PJ | 8388608 | `29101619ef0ede2fe0174ba2ab6bd2427c1fe0a1` | — |
| Power Pro Kun Pocket 6 (Japan) | BP6J | 8388608 | `43e030499a3e8ac631b3a03d78b7e52c1c7445fb` | pp6_v0_clean.gba |
| Power Pro Kun Pocket 7 (Japan) | BP7J | 8388608 | `a14cd63079c1e4652bff1f1d7fe784714341b1e9` | pp7_v0.gba |

## DS

[Catalog snapshot](https://raw.githubusercontent.com/libretro/libretro-database/90601bff67dba7057381292f17e08ed2cecbd6dc/metadat/no-intro/Nintendo%20-%20Nintendo%20DS.dat)

| Release | Code | Bytes | SHA-1 | Matching local file |
| --- | --- | ---: | --- | --- |
| Atsumare! Power Pro Kun no DS Koushien (Japan) | APXJ | 67108864 | `67ce30e66b263204aad82cfa15dccfdc47321c32` | — |
| Atsumare! Power Pro Kun no DS Koushien (Japan) (Rev 1) | APXJ | 67108864 | `ec19244d3b2f37488484c47bf1e2185054520188` | — |
| Power Pocket Koushien (Japan) | APKJ | 33554432 | `955fb91d872274d54c7f0b45e9eac1197dbdb679` | — |
| Power Pro Kun Pocket 10 (Japan) | YPJJ | 67108864 | `e8873752c143e9c1c028b089e80ee905380344f9` | — |
| Power Pro Kun Pocket 11 (Japan) | CXIJ | 67108864 | `5dd3c68686db4b108ccc70999ca9ace05f82cea4` | — |
| Power Pro Kun Pocket 12 (Japan) (NDSi Enhanced) | VPTJ | 134217728 | `4327f56f23729a59d44bd170e0ed53a2bb4667dd` | — |
| Power Pro Kun Pocket 12 (Japan) (Rev 1) (NDSi Enhanced) | VPTJ | 134217728 | `fb7239d05b85676522af977213601c206c837a0d` | — |
| Power Pro Kun Pocket 13 (Japan) (NDSi Enhanced) | VPLJ | 134217728 | `8660495da41b8172db5925213a0651ffdf9d253f` | — |
| Power Pro Kun Pocket 14 (Japan) (NDSi Enhanced) | VP4J | 134217728 | `636216f91b449aa9b132cf20ed2fa48dedb93f86` | — |
| Power Pro Kun Pocket 8 (Japan) | AP8J | 33554432 | `8dbd2336191c1ddeb653620efb2e58c965cc43eb` | — |
| Power Pro Kun Pocket 9 (Japan) | APIJ | 67108864 | `9d37ea0bbd71bda9db9bd641460061b0ce9e3aeb` | — |

## Switch

[Catalog snapshot](https://raw.githubusercontent.com/libretro/libretro-database/90601bff67dba7057381292f17e08ed2cecbd6dc/dat/Nintendo%20-%20Nintendo%20Switch.dat)

| Release | Code | Bytes | SHA-1 | Matching local file |
| --- | --- | ---: | --- | --- |
| Power Pro Kun Pocket R (Japan) | A4L4A | 7985954816 | `df20e7fb1b8bbeaedb6a51a3888fb272ee1e59bd` | — |

The [official update history](https://www.konami.com/pawa/pawapoke_r/updatelist)
lists the following software versions. Each update still needs a local retail
package to verify the decrypted content and extraction results.

| Software version | Release date | Local fixture |
| --- | --- | --- |
| [1.1.0](https://www.konami.com/pawa/pawapoke_r/update/211125) | 2021-11-25 | — |
| [1.2.0](https://www.konami.com/pawa/pawapoke_r/update/220106) | 2022-01-06 | — |
| [1.3.0](https://www.konami.com/pawa/pawapoke_r/update/220316) | 2022-03-16 | — |
| [1.4.0](https://www.konami.com/pawa/pawapoke_r/update/220415) | 2022-04-15 | — |
| [1.5.0](https://www.konami.com/pawa/pawapoke_r/update/220803) | 2022-08-03 | — |

The official [purchase bonus page](https://www.konami.com/pawa/pawapoke_r/topics/bonus)
describes Power Poke Dash for Switch as a port distributed as bonus DLC.
It needs its own fixture to determine the asset layout and verify extraction.
The GBA Dash cartridge has a separate catalog fingerprint in the GBA table.

## Additional local GBA profiles

These local ROMs are registered with the extractor and have distinct hashes from the images in the table. The two local PP5 images both have revision byte 0. Their maker codes are `CM` in `pp5_v0.gba` and `A4` in `pp5_v0_p.gba`. Neither hash matches the catalogued Rev 1. The two PP6 images both have revision byte 0. Restoring bytes `07 da` over `00 00` at file offset `0x44246` in `pp6_v0.gba` produces `pp6_v0_clean.gba`, which matches the catalogued CRC32, MD5, and SHA-1.

| Local file | SHA-1 |
| --- | --- |
| pp1_2_v0_p.gba | `43badcefe8cad127f8f8655ee89262517399a20b` |
| pp5_v0_p.gba | `f9fd4c8ee4f274c751551f056dc868abd0355390` |
| pp6_v0.gba | `066d2086d776c66aa566eac211a0e9b45f1942f6` |
