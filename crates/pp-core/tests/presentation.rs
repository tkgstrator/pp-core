use pp_core::presentation::{Cue, Presentation, Side};
use pp_core::script::Script;

#[test]
fn emits_presentation_cues_in_script_order_and_keeps_state_between_scenes() {
    let script = Script::from_yaml(
        r#"- scene: before
  commands:
    - type: background
      asset: gate
    - type: left
      asset: protagonist
    - type: bgm
      action: start
      asset: after_school
    - type: go
      scene: after
- scene: after
  commands:
    - type: background
      asset: street
    - type: se
      asset: thunder
    - type: effect
      effect: shake
    - type: wait
      ms: 400
    - type: bgm
      action: start
      asset: rain
    - type: left
      asset: null
    - type: bgm
      action: stop
"#,
    )
    .unwrap();
    let mut presentation = Presentation::default();
    let before = &script.scene("before").unwrap().commands;
    assert_eq!(
        before
            .iter()
            .filter_map(|cmd| presentation.apply(cmd))
            .collect::<Vec<_>>(),
        [
            Cue::SetBackground { asset: "gate" },
            Cue::SetPortrait {
                side: Side::Left,
                asset: Some("protagonist")
            },
            Cue::StartBgm {
                asset: "after_school"
            },
        ]
    );
    assert_eq!(presentation.background(), Some("gate"));
    assert_eq!(presentation.bgm(), Some("after_school"));

    let after = &script.scene("after").unwrap().commands;
    assert_eq!(
        presentation.apply(&after[0]),
        Some(Cue::SetBackground { asset: "street" })
    );
    assert_eq!(presentation.background(), Some("street"));
    assert_eq!(presentation.portrait(Side::Left), Some("protagonist"));
    assert_eq!(presentation.bgm(), Some("after_school"));
    assert_eq!(
        presentation.apply(&after[1]),
        Some(Cue::PlaySe { asset: "thunder" })
    );
    assert_eq!(
        presentation.apply(&after[2]),
        Some(Cue::PlayEffect { effect: "shake" })
    );
    assert_eq!(presentation.apply(&after[3]), Some(Cue::Wait { ms: 400 }));
    assert_eq!(
        presentation.apply(&after[4]),
        Some(Cue::StartBgm { asset: "rain" })
    );
    assert_eq!(presentation.bgm(), Some("rain"));
    assert_eq!(
        presentation.apply(&after[5]),
        Some(Cue::SetPortrait {
            side: Side::Left,
            asset: None
        })
    );
    assert_eq!(presentation.portrait(Side::Left), None);
    assert_eq!(presentation.apply(&after[6]), Some(Cue::StopBgm));
    assert_eq!(presentation.bgm(), None);
}
