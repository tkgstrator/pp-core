use pp_core::script::{Bgm, Command, Script};

#[test]
fn reads_the_complete_documented_scenario() {
    let yaml = include_str!("../../../docs/pp-core/scripts/format.md").replace("\r\n", "\n");
    let yaml = yaml
        .split_once("```yaml\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    let script = Script::from_yaml(yaml).unwrap();
    assert_eq!(script.scenes().len(), 2);
    assert!(script.verify().is_ok());
    let first = script.scene("after_practice").unwrap();
    assert_eq!(first.when.map(u32::from), Some(12));
    assert_eq!(first.prob, Some(30));
    assert!(
        matches!(&first.commands[3], Command::Bgm(Bgm::Start { asset }) if asset == "after_school")
    );
    assert!(
        matches!(&first.commands[4], Command::Text { speaker: Some(speaker), text } if speaker == "hazuki" && text[0].contains("{NAME}"))
    );
    assert!(matches!(
        &first.commands[5],
        Command::Text { speaker: Some(speaker), .. } if speaker == "protagonist"
    ));
    assert!(script.scene("walk_to_station").is_some());
}

#[test]
fn rejects_invalid_commands_and_scene_links() {
    for commands in [
        "  - type: unknown\n",
        "  - type: text\n    text: []\n",
        "  - type: text\n    label: 葉月\n    text: [こんにちは]\n",
        "  - type: wait\n    ms: -1\n",
        "  - type: left\n",
        "  - type: bgm\n    action: start\n",
        "  - type: bgm\n    action: stop\n    asset: theme\n",
        "  - type: set\n    flag: ready\n    value: 5\n",
        "  - type: if\n    condition:\n      flag: ready\n      gt: true\n    then: []\n",
        "  - type: if\n    condition:\n      stat: hp\n      eq: 1\n      ge: 2\n    then: []\n",
        "  - type: switch\n    stat: hp\n    cases:\n      - case:\n          value: true\n          commands: []\n",
        "  - type: choice\n    options:\n      - text: Next\n        commands:\n          - type: go\n            scene: missing\n",
    ] {
        let yaml = format!("- scene: opening\n  commands:\n{commands}");
        assert!(Script::from_yaml(&yaml).is_err(), "{commands}");
    }
}

#[test]
fn rejects_invalid_scene_headers() {
    for yaml in [
        "- scene: start\n  when: 0\n  commands: []\n",
        "- scene: start\n  prob: 101\n  commands: []\n",
        "- scene: start\n  commands: []\n- scene: start\n  commands: []\n",
        "format_version: 2\nscene_id: opening\ncommands: []\n",
    ] {
        assert!(Script::from_yaml(yaml).is_err(), "{yaml}");
    }
}

#[test]
fn verifies_reachability_without_following_dead_jumps() {
    let script = Script::from_yaml(
        r#"- scene: start
  commands:
    - type: if
      condition:
        flag: ready
        eq: true
      then:
        - type: go
          scene: linked
      else:
        - type: end
    - type: go
      scene: dead
- scene: linked
  commands:
    - type: go
      scene: start
- scene: dead
  commands: []
- scene: scheduled
  when: 12
  commands: []
- scene: disabled
  prob: 0
  commands: []
"#,
    )
    .unwrap();
    let report = script.verify();
    assert_eq!(report.unreachable_scenes, ["dead", "disabled"]);
    assert_eq!(report.unreachable_commands.len(), 1);
    assert_eq!(report.unreachable_commands[0].scene, "start");
    assert_eq!(report.unreachable_commands[0].path, "commands[1]");
}

#[test]
fn recognizes_exhaustive_switch_and_shadowed_cases() {
    let script = Script::from_yaml(
        r#"- scene: start
  commands:
    - type: switch
      flag: ready
      cases:
        - case:
            value: true
            commands:
              - type: end
        - case:
            value: false
            commands:
              - type: end
        - case:
            value: true
            commands:
              - type: go
                scene: hidden
      default:
        - type: go
          scene: hidden
    - type: go
      scene: hidden
- scene: hidden
  commands: []
"#,
    )
    .unwrap();
    let report = script.verify();
    assert_eq!(report.unreachable_scenes, ["hidden"]);
    assert_eq!(
        report
            .unreachable_commands
            .iter()
            .map(|item| item.path.as_str())
            .collect::<Vec<_>>(),
        [
            "commands[0].cases[2].case.commands[0]",
            "commands[0].default[0]",
            "commands[1]"
        ]
    );
}
