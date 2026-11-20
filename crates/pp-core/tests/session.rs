use pp_core::script::{Script, Value};
use pp_core::session::{Event, Pause, Scenario, Session};

#[test]
fn plays_the_documented_scenario_through_choice_wait_and_scene_change() {
    let source = include_str!("../../../docs/pp-core/scripts/format.md").replace("\r\n", "\n");
    let source = source
        .split_once("```yaml\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    let scenario = Scenario::from_yaml(
        "start_scene: after_practice\nmax_turns: 144\nphases: [朝, 昼, 夜]\ncharacters:\n  hazuki: 葉月\nstats:\n  trust:\n    initial: 48\n    max: 55\nflags:\n  club_member: true\n  walked_to_station: false\n"
    ).unwrap();
    let mut session = Session::new(Script::from_yaml(source).unwrap(), scenario, "あおい").unwrap();
    assert!(session.trigger(12, 2, || 99).unwrap().is_empty());
    assert!(matches!(session.view().pause, Pause::Ended));
    assert_eq!(
        session.trigger(12, 2, || 0).unwrap(),
        [Event::StartBgm {
            asset: "after_school".into()
        }]
    );
    assert!(
        matches!(session.view().pause, Pause::Text { speaker: Some(name), text } if name == "葉月" && text.starts_with("あおい"))
    );
    session.advance().unwrap();
    assert!(
        matches!(session.view().pause, Pause::Text { speaker: Some(name), .. } if name == "あおい")
    );
    for _ in 0..3 {
        assert!(session.advance().unwrap().is_empty());
    }
    assert!(matches!(session.view().pause, Pause::Choice { .. }));
    assert!(session.choose(99).is_err());
    session.choose(0).unwrap();
    for _ in 0..3 {
        session.advance().unwrap();
    }
    assert_eq!(
        session.advance().unwrap(),
        [
            Event::PlaySe {
                asset: "thunder".into()
            },
            Event::PlayEffect {
                effect: "shake".into()
            },
        ]
    );
    let view = session.view();
    assert_eq!(view.scene, Some("walk_to_station"));
    assert_eq!(view.left, Some("protagonist"));
    assert_eq!(view.background, Some("shopping_street_night"));
    assert_eq!(view.bgm, Some("after_school"));
    assert!(matches!(view.pause, Pause::Wait { ms: 400 }));
    assert!(session.advance().is_err());
    assert!(session.resume().unwrap().is_empty());
    assert!(matches!(session.view().pause, Pause::Text { text, .. } if text.starts_with("雷だ")));
    for _ in 0..5 {
        session.advance().unwrap();
    }
    assert_eq!(session.advance().unwrap(), [Event::StopBgm]);
    assert!(matches!(session.view().pause, Pause::Ended));
}

#[test]
fn rejects_non_yielding_loops() {
    let script =
        Script::from_yaml("- scene: loop\n  commands:\n    - type: go\n      scene: loop\n")
            .unwrap();
    let scenario = Scenario::from_yaml("start_scene: loop\nmax_turns: 1\nphases: [朝]\n").unwrap();
    let mut session = Session::new(script, scenario, "あおい").unwrap();
    assert!(session.start("loop").is_err());
    assert!(matches!(session.view().pause, Pause::Ended));
}

#[test]
fn configured_start_and_numeric_schedule_are_validated() {
    assert!(Scenario::from_yaml("start_scene: opening\nphases: [朝]\n").is_err());
    assert!(Scenario::from_yaml("start_scene: opening\nmax_turns: 0\nphases: [朝]\n").is_err());
    let yaml = "start_scene: opening\nmax_turns: 2\nphases: [朝, 夜]\n";
    let script = Script::from_yaml(
        "- scene: morning\n  when: 2\n  phase: 0\n  commands: []\n- scene: opening\n  commands: []\n",
    ).unwrap();
    let mut session = Session::new(script, Scenario::from_yaml(yaml).unwrap(), "あおい").unwrap();
    session.start_configured().unwrap();
    assert_eq!(session.view().scene, Some("opening"));
    assert!(session.trigger(3, 0, || 0).is_err());
    assert!(session.trigger(2, 2, || 0).is_err());
    session.trigger(2, 0, || 0).unwrap();
    assert_eq!(session.view().scene, Some("morning"));

    let invalid = Script::from_yaml(
        r#"- scene: opening
  when: 3
  phase: 2
  commands:
    - type: choice
      options:
        - text: Speak
          commands:
            - type: text
              speaker: missing
              text:
                - Hello
"#,
    )
    .unwrap();
    let errors = Scenario::from_yaml(yaml)
        .unwrap()
        .verify(&invalid)
        .unwrap_err();
    for expected in ["when", "phase", "options[0].commands[0]", "missing"] {
        assert!(errors.join("\n").contains(expected), "{errors:?}");
    }
}

#[test]
fn records_each_applied_state_change_even_when_the_final_value_is_unchanged() {
    let scenario = Scenario::from_yaml(
        r#"start_scene: opening
max_turns: 1
phases: [朝]
stats:
  trust:
    initial: 48
    max: 50
flags:
  met: false
"#,
    )
    .unwrap();
    let script = Script::from_yaml(
        r#"- scene: opening
  commands:
    - type: add
      stat: trust
      value: 5
    - type: sub
      stat: trust
      value: 2
    - type: set
      flag: met
      value: true
    - type: set
      flag: met
      value: true
    - type: text
      text:
        - First
        - Second
"#,
    )
    .unwrap();
    let mut session = Session::new(script, scenario, "あおい").unwrap();
    session.start_configured().unwrap();
    let changes = session.state_changes();
    assert_eq!(changes.len(), 3);
    assert_eq!(
        (changes[0].before, changes[0].after),
        (Value::Integer(48), Value::Integer(50))
    );
    assert_eq!(
        (changes[1].before, changes[1].after),
        (Value::Integer(50), Value::Integer(48))
    );
    assert_eq!(
        (changes[2].before, changes[2].after),
        (Value::Boolean(false), Value::Boolean(true))
    );
    assert_eq!(session.view().stats["trust"], 48);
    session.advance().unwrap();
    assert!(session.state_changes().is_empty());
}
