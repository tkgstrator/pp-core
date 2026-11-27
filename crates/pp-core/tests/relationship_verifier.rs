use pp_core::script::Script;

#[test]
fn checks_actual_branch_continuations_without_borrowing_another_paths_change() {
    let script = Script::from_yaml(
        r#"
- scene: start
  commands:
    - type: choice
      options:
        - text: 一人で戻る
          commands: []
        - text: 別室へ行く
          commands:
            - type: text
              speaker: friend_alias
              text: [おつかれ。]
            - type: go
              scene: supported
        - text: 立ち話をする
          commands:
            - type: text
              speaker: friend
              text: [元気か。]
            - type: sub
              stat: bond
              value: 0
            - type: end
    - type: add
      stat: bond
      value: 1
- scene: supported
  commands:
    - type: sub
      stat: bond
      value: 1
    - type: effect
      effect: result:other
    - type: go
      scene: next_event
- scene: next_event
  commands:
    - type: text
      speaker: friend
      text: [またね。]
"#,
    )
    .unwrap();
    let speakers = [
        ("friend".into(), "bond".into()),
        ("friend_alias".into(), "bond".into()),
    ]
    .into();
    let warnings = script.verify_relationship_changes(&speakers, Some("result:"));
    assert_eq!(warnings.len(), 2);
    assert!(warnings.iter().any(|warning| {
        warning
            .choice
            .as_ref()
            .is_some_and(|choice| choice.label == "立ち話をする")
    }));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.appearance.scene == "next_event" && warning.choice.is_none())
    );
}

#[test]
fn a_shared_effect_after_a_choice_covers_every_path_that_reaches_it() {
    let script = Script::from_yaml(
        r#"
- scene: start
  commands:
    - type: choice
      options:
        - text: 呼ぶ
          commands:
            - type: text
              speaker: friend
              text: [なに？]
        - text: 帰る
          commands: []
    - type: add
      stat: bond
      value: 1
"#,
    )
    .unwrap();
    let speakers = [("friend".into(), "bond".into())].into();
    assert!(
        script
            .verify_relationship_changes(&speakers, None)
            .is_empty()
    );
}
