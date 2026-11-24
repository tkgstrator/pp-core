# 拡張 PP スクリプト

拡張 PP スクリプトは、パワポケの会話イベントをベースにした `pp-core` の YAML 形式です。キーとコマンド名は小文字の snake_case で記述します。抽出 CLI が出力する [YAML v2](../../script-format.md) とは互換性を持ちません。

## シーン

YAML のルートはシーンの配列です。各シーンに一意の `scene` ID と、上から順に実行する `commands` 配列を指定します。各項目の `type` で命令を指定します。会話は話者の発言ごとに `type: text` の1項目にまとめ、その中に表示順でセリフを並べます。演出や分岐も同じ配列に実行順で並べます。配列とマッピングはインデントで展開します。

主人公と葉月が放課後に話す、この仕様用のオリジナルシーンです。イベントの入口条件と、この文書で定義する全コマンドを含みます。整数の `trust`、真偽値の `club_member` と `walked_to_station`、例で参照する素材をシナリオ側で用意します。

```yaml
- scene: after_practice
  when: 12
  phase: 2
  if:
    flag: club_member
    eq: true
  prob: 30
  commands:
    - type: background
      asset: school_gate_evening
    - type: left
      asset: protagonist
    - type: right
      asset: hazuki
    - type: bgm
      action: start
      asset: after_school

    - type: text
      speaker: hazuki
      text:
        - "{NAME}さん、今帰るところ？"

    - type: text
      speaker: protagonist
      text:
        - うん。片づけを手伝ってたら、すっかり遅くなった。

    - type: text
      speaker: hazuki
      text:
        - 私も図書室にいたら、閉館の放送でようやく気づいたよ。
        - 駅まで一緒に歩かない？

    - type: choice
      options:
        - text: 一緒に帰ろう
          commands:
            - type: text
              speaker: protagonist
              text:
                - いいよ。商店街を通っていこうか。

            - type: text
              speaker: hazuki
              text:
                - うん。あっちなら、まだお店の明かりがついてるね。
            - type: add
              stat: trust
              value: 5

            - type: if
              condition:
                stat: trust
                ge: 50
              then:
                - type: text
                  speaker: hazuki
                  text:
                    - "{NAME}さんと話してると、駅まであっという間だね。"
                    - 明日も時間が合ったら、一緒に帰ろうよ。
              else:
                - type: text
                  speaker: hazuki
                  text:
                    - 一緒に帰るの、今日が初めてだね。

            - type: go
              scene: walk_to_station
        - text: 先に帰ってて
          commands:
            - type: text
              speaker: protagonist
              text:
                - ごめん。部室に水筒を忘れたみたいだ。

            - type: text
              speaker: hazuki
              text:
                - そっか。じゃあ、また明日ね。
            - type: sub
              stat: trust
              value: 1
            - type: right
              asset: null

    - type: text
      speaker: protagonist
      text:
        - また明日。気をつけて。
    - type: bgm
      action: stop
    - type: end

- scene: walk_to_station
  commands:
    - type: background
      asset: shopping_street_night

    - type: se
      asset: thunder
    - type: effect
      effect: shake
    - type: wait
      ms: 400

    - type: switch
      flag: walked_to_station
      cases:
        - case:
            value: true
            commands:
              - type: text
                speaker: hazuki
                text:
                  - 前に一緒に帰った日も、こんな空だったね。
      default:
        - type: text
          speaker: protagonist
          text:
            - 雷だ。雨が降る前に駅に着けるといいね。

    - type: text
      speaker: hazuki
      text:
        - あのパン屋さん、夕方でもまだ開いてるんだ。

    - type: text
      speaker: protagonist
      text:
        - 練習の帰りによく寄るよ。売り切れてなければ、あんパンがおすすめ。

    - type: text
      speaker: hazuki
      text:
        - じゃあ明日、一本早い電車で来て寄ってみようかな。

    - type: text
      speaker: protagonist
      text:
        - 朝は駅前の店のほうが早く開くよ。学校までの通り道だし。

    - type: text
      speaker: hazuki
      text:
        - ありがとう。明日、教室で感想を言うね。
    - type: set
      flag: walked_to_station
      value: true
    - type: bgm
      action: stop
    - type: end
```

## イベントの発火

| シーン直下のキー | 意味 |
| --- | --- |
| `when` | ターン番号。1 から `scenario.yaml` の `max_turns` までの整数。 |
| `phase` | ターン内のフェーズ ID。`scenario.yaml` の `phases` の配列インデックス（0 始まり）。 |
| `if` | フラグや数値パラメータを調べる入口条件。 |
| `prob` | 発火確率。0〜100 の百分率。省略時は 100。 |

イベント探索の対象は、上表のキーを少なくとも一つ指定したシーンです。指定された `when`、`phase`、`if` をすべて満たした候補だけ、探索1回につき1回 `prob` を判定します。判定を通ったシーンが配列順で最初に見つかった時点で、そのシーンを開始します。入口条件や確率を通らなかったシーンのコマンドは実行されません。

`go` は行き先を直接開始します。行き先の入口条件や確率は判定しません。必ず発生する場面で条件に応じた展開を見せる場合は、シーン内の `if` を使います。

## 会話と演出

| コマンド・記法 | 引数と動作 |
| --- | --- |
| `text` | `speaker` にキャラクター ID、`text` に1件以上のセリフを指定する。セリフを1件ずつ表示し、表示ごとにプレイヤーが次へ進むまで待つ。`speaker` を省略すると名前欄を表示しない。 |
| `{NAME}` | `text` 内で使えるプレースホルダー。実行時のプレイヤー名で置き換える。 |
| `left` / `right` | `asset` に左・右のキャラ素材 ID を指定する。`asset: null` で消す。 |
| `background` | `asset` に背景素材 ID を指定すると即時に切り替える。 |
| `wait` | `ms` に非負整数のミリ秒を指定する。指定時間が経過したら自動で進む。 |
| `effect` | `effect` に演出 ID を指定する。例は `shake`。完了を待つ場合は後続の `wait` で指定する。 |
| `se` | `asset` に効果音素材 ID を指定する。再生を開始して次へ進む。 |
| `bgm` | `action` に `start` を指定すると、`asset` の曲を再生して再生中の曲を置き換える。`stop` を指定すると停止する。 |

再生側は `speaker` をキャラクター定義の表示名に解決します。サンプルの `hazuki` は「葉月」、`protagonist` はプレイヤーが入力した名前を表示します。立ち絵の `asset` には画像の素材 ID を指定します。

背景、立ち絵、BGM の表示・再生状態は、後続の命令で変更されるまで維持します。

Rust の再生側は、到達した命令を `pp_core::presentation::Presentation::apply` に渡して、描画や音声出力に使う `Cue` を受け取れます。同じ `Presentation` をシーン間で使い続けると、背景・立ち絵・BGM の状態を保持します。SE とエフェクトは開始の指示を返し、`wait` の指示が来たときは再生側が指定時間の経過後に進行を再開します。

## 分岐と終了

| コマンド | 引数と動作 |
| --- | --- |
| `choice` | `options` に、各項目が `text` と `commands` を持つ配列を指定する。プレイヤーの選択を待ち、選んだ項目の命令を実行する。 |
| `if` | `condition`、`then`、任意の `else` を持つ。条件に対応する命令配列を実行する。 |
| `switch` | 参照する `stat` または `flag` と、`cases` 配列を指定する。任意の `default` は一致する項目がないときに実行する命令配列。 |
| `case` | `cases` の項目に使うキー。`value` と `commands` を指定し、値が型を含めて一致した最初の項目を実行する。 |
| `go` | `scene` に移動先のシーン ID を指定する。残りの命令を打ち切り、移動先の先頭から実行する。 |
| `end` | 現在のシーンを終了する。引数は不要。シーン直下の命令配列を最後まで実行した場合も終了する。 |

選択肢や条件分岐の命令配列を最後まで実行したら、親の配列の次の命令に進みます。入れ子の中でも `go` と `end` はシーン全体に作用します。

条件は `stat` または `flag` のどちらか一つと、比較演算子一つを指定します。整数には `eq`、`ne`、`lt`、`le`、`gt`、`ge` を、真偽値には `eq`、`ne` を使えます。入口の `if` とコマンドの `condition` は同じ書式です。

シーン入口の `if` には条件の配列も書けます。配列のときは、すべての条件を満たした場合だけ候補になります。前のイベントを見たことを示すフラグと組み合わせると、イベントを順番に発生させられます。

```yaml
- scene: second_visit
  if:
    - flag: seen_first_visit
      eq: true
    - stat: trust
      ge: 50
  commands:
    - type: text
      text:
        - また来てくれたんだ。
```

```yaml
- type: if
  condition:
    stat: trust
    ge: 50
  then:
    - type: switch
      flag: walked_to_station
      cases:
        - case:
            value: true
            commands:
              - type: text
                text:
                  - この前の話、覚えているか？
      default:
        - type: go
          scene: walk_to_station
  else:

    - type: text
      text:
        - 今日の練習はここまでだ。
```

## 状態の更新

| コマンド | 引数と動作 |
| --- | --- |
| `set` | `stat` または `flag` を指定し、`value` の整数または真偽値を設定する。 |
| `add` | `stat` に `value` の非負整数を加える。 |
| `sub` | `stat` から `value` の非負整数を引く。 |

シナリオの状態定義で、パラメータとフラグの ID、型、初期値を指定します。整数パラメータは任意で `min`、`max` を持ち、更新後の値をその範囲に収めます。更新はその場で反映し、後続の条件判定から参照できます。未定義の状態 ID やシーン ID、型の不一致、未知の命令はスクリプトのエラーです。状態の初期値は `scenario.yaml` に定義します。

## Rust からの読み込みと検証

`pp_core::script::Script::from_yaml` に YAML 文字列を渡すと、順序を保持したシーンとコマンドを読み込めます。読み込み時にコマンドの引数、重複したシーン ID、存在しない `go` の行き先を検証します。`Scenario::verify` は設定と合わせて、状態 ID、キャラクター ID、ターンとフェーズの範囲を検証します。素材 ID は再生側が解決します。

```rust
use pp_core::{script::Script, session::Scenario};

let script = Script::from_yaml(source)?;
let scenario = Scenario::from_yaml(config)?;
let report = scenario.verify(&script).map_err(|errors| errors.join("\n"))?;
assert!(report.is_ok(), "{report:?}");
```

`Scenario::verify` は `start_scene` と、発火条件を持つ `prob: 0` 以外のシーンを入口として調べます。シーン間の移動は `go` をたどります。到達できないシーン ID と、到達できない命令の位置（例 `commands[3].else[0]`）を報告します。位置はゼロ始まりです。シーン全体に到達できない場合はシーン ID を報告します。

この検証ではプレイヤーの選択やフラグの値を可能な範囲で分岐として扱います。状態更新の結果や、同一フェーズで優先されるイベントの判定にはシナリオの実行が必要です。

## シナリオ設定

スクリプトとは別に [`scenario.yaml`](../../../web/scenario.yaml) が必要です。必須項目は以下の3つです。

```yaml
start_scene: after_practice
max_turns: 144
phases:
  - 朝
  - 昼
  - 夜
```

- `start_scene` は開始するシーンの ID です。
- `max_turns` は正の整数です。ターン番号は 1 から始まります。
- `phases` は各ターン内のフェーズの表示名を実行順に並べた配列です。フェーズ数は配列の長さで決まり、上の例では朝が `0`、昼が `1`、夜が `2` です。スクリプトはこのインデックスを参照するので、配列の順序を変える場合は `phase` の指定も更新します。

`characters`、`stats`、`flags` は任意です。スクリプトで使うキャラクターの表示名、数値の初期値と範囲、フラグの初期値を定義します。プレイヤー名はセッション開始時に別途渡します。

## 再生セッション

`pp_core::session::Session` はスクリプト、進行位置、表示状態、シナリオの状態を保持します。セッション作成時に `scenario.yaml` とスクリプトの参照を検証します。

`start_configured` は設定の開始シーンを開きます。`trigger` はターン番号とフェーズ ID を指定して対象イベントを探します。Web UI の開始ボタンは設定の開始シーンを使います。進行中は `advance` でセリフを送り、`choose` に選択肢のゼロ始まりの番号を渡し、`resume` で待機を解除します。WASM の各操作は、更新後の状態と、その操作で発生した音声・エフェクトのイベントを返します。

育成シミュレーションなど、ホスト側が持つ値は `set_stat` と `set_flag` でシナリオに書き込めます。値は設定の範囲に丸められ、スクリプトによる変更履歴には記録されません。WASM の `trigger_seeded` は `trigger` と同じ探索を、シードから決まる確率判定で行います。同じシードと入力の列で、同じ進行を再生できます。`view` はコマンドを実行せずに現在の状態を返します。
