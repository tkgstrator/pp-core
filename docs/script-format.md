# Generic dialogue scene YAML (version 2)

`<output>/4/scripts/scenes/` and `<output>/6/scripts/scenes/` contain the primary, game-independent scene format. A consumer reads `index.tsv`, opens a scene file, and executes its `commands` in order. Each scene has an opaque `scene_id`; each command has an opaque `label`. Neither ID is a ROM address. IDs are deterministic for the two verified ROM revisions and this extractor version; consumers should treat them as identifiers rather than deriving meaning from their spelling.

```yaml
---
format_version: 2
game: "pp6"
scene_id: "pp6_scene_000001"
commands:
  -
    label: "c000001"
    type: "IF"
    variable: "pp6_state_000024"
    operator: "bits_set"
    value: 255
    value_type: "u8"
    false_target: "c000004"
  -
    label: "c000002"
    type: "EFFECT"
    effect: "background"
    asset_id: 12
```

The generic YAML has no `offset`, `bytes_hex`, or `args` keys. `index.tsv` links each scene ID to its generic file and its corresponding lossless file in `native/`. The commands in the two files have the same order, so their list positions also provide an instruction link. `state-links.tsv` maps opaque state IDs to native references for investigation; a generic consumer need not read it. The complete previous YAML, text listings, TSV indexes, and JSON summaries remain byte-for-byte unchanged under `scripts/native/`.

## Commands

| Type | Named fields | Meaning verified from the existing decoders |
| --- | --- | --- |
| `TEXT` | `text` | Display Japanese dialogue text. |
| `CHOICE` | `options` (ordered strings) | Display the options. The destination selected by an option is not encoded in this command. |
| `LINE`, `PAGE` | none | Advance a line or page. |
| `WAIT` | `until: "page_ack"` | PP6 page acknowledgment wait. The analogous PP4 command is not verified. |
| `EFFECT` | `effect: "background"`, `asset_id` | Change displayed background or scene image. |
| `EFFECT` | `effect: "portrait"`, `side`, `asset_id` | Change left or right portrait. |
| `EFFECT` | `effect: "text_style"`, `mode` | PP4 style on/restore or PP6 emphasis/restore. |
| `PLAYER_NAME` | none | Insert the protagonist name. |
| `IF` | `variable`, `operator`, `value`, `value_type`, `false_target` | Compare state and branch when false. `bits_set` means the masked result is nonzero. PP4 32-bit values follow the source decoder's `u32`; PP6 follows its `s32`. |
| `SWITCH` | `variable`, `cases` (`value`, `target`) | Select a labeled branch using a runtime byte. |
| `SET`, `ADD`, `SUBTRACT`, `BIT_AND`, `BIT_OR` | `variable`, `value_type`, `value` | PP6 state mutation with an immediate value. |
| `COPY` | `variable`, `value_type`, `source` | PP6 state copy between opaque state IDs. |
| `CALL` | `target` | Call another scene and retain the return position. |
| `JUMP` | `target` | Transfer to a scene or a command label without retaining a return position. |
| `END_OR_RETURN` | none | Terminal VM byte. Whether it ends the scene or returns depends on runtime call context. |
| `MARKER` | none | Structural branch marker; execution proceeds to the next command. |
| `INLINE_DATA` | `purpose` | Bytes after PP4's terminal indexed jump; not an executable command. |
| `UNRESOLVED` | `reason: "unverified_semantics"` | Meaning has not been verified. A player must handle this explicitly; skipping it can change state or presentation. |

`target` and `false_target` use a command label in the current scene, a scene ID, or `END` (the position after the final command). `variable` and `source` are opaque global state IDs within a game, not descriptor-local slot numbers. State IDs are assigned by sorted native references so aliases across scenes share one ID.

The schema reserves `SE` (`action: "play"`, `asset_id`), `BGM` (`action: "play"` or `"stop"`, with `asset_id` for play), and `RETURN` (no fields). No current generic scene emits them. The audio extraction work verifies BGM and SE asset IDs, but the available script research does not establish which dialogue VM opcode calls those dispatchers. The terminal byte is documented as end *or* return, so emitting `RETURN` for it would claim a distinction the source does not support.

This format supports rendering the verified dialogue and following known static control flow. It does not yet prove complete game playback: many extended instructions have unknown effects, some choice-result behavior remains tied to runtime state, and SE/BGM triggers have no verified script mapping. Inspect the linked native record when an `UNRESOLVED` command matters to a player.
