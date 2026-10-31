# voicevox-cli

**日本語 | [English](README.md)**

`vvsay` は [VOICEVOX CORE](https://github.com/VOICEVOX/voicevox_core) を使った読み上げコマンドです。プリセット管理と自動再生に対応しています。

## 特徴

- 🎵 **mpv による自動再生** - 出力ファイルを指定しなければその場で再生
- ⚡ **すぐに鳴る** - 文の長さに関係なく約0.6秒で再生開始。音声を少しずつ生成してプレイヤーに流し、再生と並行して続きを合成します
- 📝 **音声プリセット** - キャラクター・スタイル・パラメータの組み合わせを保存
- 📜 **長文対応** - 文字数制限なし。長い文は区切って合成し、メモリ使用量を抑えます
- 🧾 **順番どおりの再生** - `--queue` で他の再生の終了を待ち、呼び出し順に再生
- 🔄 **パイプ入力対応** - `echo "テキスト" | vvsay`
- 🚀 **バックグラウンド実行** - `--bg` で即座にシェルへ制御を返す
- ⚙️ **設定ファイル** - `~/.config/vvsay/config.toml`

## 動作要件

- macOS または Linux。Linux は arm64 で検証済みです（Debian 12 でインストール、`--setup`、合成、PulseAudio での再生）。x64 は未検証です
- 再生用の [mpv](https://mpv.io/)（`brew install mpv`、またはディストリビューションの `mpv` パッケージ）。Linux では、デスクトップのセッションの PipeWire または PulseAudio で再生します
- ビルド用の Rust 1.89 以上、C++ コンパイラ、[CMake](https://cmake.org/)（`brew install cmake`、Debian/Ubuntu では `build-essential` と `cmake`）
- VOICEVOX CORE の実行ファイル類（後述）

## インストール

```bash
cargo install --git https://github.com/petamorikei/voicevox-cli
```

ローカルのクローンからは次のとおりです。

```bash
cargo install --path .
```

`voicevox_core` が crates.io で公開されていないため、crates.io ではなく git からインストールします。

### VOICEVOX CORE の実行ファイル類のダウンロード

`vvsay` には VOICEVOX ONNX Runtime、Open JTalk 辞書、音声モデル（VVM ファイル）が必要です。
ターミナルで次のコマンドを実行してください。

```bash
vvsay --setup
```

公式の VOICEVOX CORE ダウンローダー（0.17.1、チェックサム検証済み）を取得して実行します。
**ダウンローダーが ONNX Runtime と音声モデルの利用規約への同意を求めるので、内容を読んでご自身で回答してください。**
通常は vvsay が代わりに回答することはありません。規約を読んで同意したうえで自動化する場合（CI など）は `vvsay --setup --accept-terms` を使ってください。規約の本文は表示されたまま、確認に vvsay が「同意する」と回答します。導入済みの場合は、再ダウンロードの確認に標準入力から yes を渡さない限り何もしません。

- 保存先は `~/.local/share/vvsay`（設定ファイルの `assets_dir` で変更可能）です。
- トーク用モデルだけを取得します（ソング用は使いません）。
- GitHub のレートリミットに当たる場合は `GH_TOKEN=$(gh auth token) vvsay --setup` で実行してください。
- 導入済みの状態で実行すると、再ダウンロードするか確認されます。

<details>
<summary>手動で導入する場合</summary>

```bash
mkdir -p ~/.local/share/vvsay && cd ~/.local/share/vvsay
curl -sSfL https://github.com/VOICEVOX/voicevox_core/releases/download/0.17.1/download-osx-arm64 -o download
chmod +x download
./download --only onnxruntime dict models --models-pattern '[0-9]*.vvm' -o ~/.local/share/vvsay
```

Intel Mac では `download-osx-x64`、Linux では `download-linux-x64` / `download-linux-arm64` を使ってください。
</details>

### 利用規約

音声モデルには **VOICEVOX 音声モデル利用規約**（`~/.local/share/vvsay/models/TERMS.txt`）と、各音声ライブラリ（キャラクター）の利用規約が適用されます。
多くのキャラクターでは、生成した音声を公開する際に `VOICEVOX:ずんだもん` のようなクレジット表記が必要です。公開前に各キャラクターの規約を確認してください。

## 使用方法

```bash
# キャラクターを指定して読み上げ（-s 省略時はそのキャラの最初のスタイル）
vvsay -n ずんだもん "こんにちは、ずんだもんなのだ。"

# スタイルを指定
vvsay -n ずんだもん -s あまあま "こんにちは"

# スタイル ID で指定
vvsay --style-id 1 "こんにちは"

# 再生せず WAV ファイルに保存
vvsay -n ずんだもん -o output.wav "こんにちは"

# ファイルやパイプから入力
vvsay -n ずんだもん -t input.txt
cat document.txt | vvsay -p zundamon

# 声の調整
vvsay -n ずんだもん --speed 1.2 --pitch 0.05 --intonation 1.2 --volume 1.0 "こんにちは"

# キャラクター・スタイル・プリセットの一覧
vvsay --list-narrator
vvsay --list-style ずんだもん
vvsay --list-presets

# バックグラウンド実行
vvsay --bg -p zundamon "ビルドを開始するのだ"

# 重ならないよう呼び出し順に再生
vvsay --bg --queue -p zundamon "ビルドを開始するのだ"
vvsay --bg --queue -p zundamon "ビルドが完了したのだ"
```

### 再生キュー

| 呼び出し方 | 動作 |
|---|---|
| `--queue` なし（デフォルト） | すぐに再生します。他の再生と重なることがあります。 |
| `--queue` あり | 合成はすぐに始めますが、他の再生（`--queue` の有無を問わず）がすべて終わってから再生します。`--queue` 同士は呼び出した順に再生されます。 |

`-o` を指定した場合、`--queue` は効果がありません。

## 設定

`~/.config/vvsay/config.toml` は初回実行時に作成されます。

```toml
default_preset = "zundamon"
# assets_dir = "~/.local/share/vvsay"

[[presets]]
name = "zundamon"
narrator = "ずんだもん"
style = "ノーマル"
speed = 1.1

[[presets]]
name = "metan-whisper"
narrator = "四国めたん"
style = "ささやき"
pitch = 0.03
intonation = 1.2
```

- `default_preset`: `-p` を省略したときに使うプリセット
- `assets_dir`: VOICEVOX CORE の実行ファイル類の場所（デフォルト `~/.local/share/vvsay`）
- `presets[].narrator`: `--list-narrator` で表示されるキャラクター名
- `presets[].style`: スタイル名（省略時はそのキャラの最初のスタイル）
- `presets[].speed` / `pitch` / `intonation` / `volume` / `pre_phoneme_length` / `post_phoneme_length`: 任意。コマンドラインオプションと同じ意味

### パラメータの優先順位

1. コマンドラインオプション
2. プリセット（`-p`、または `default_preset`）
3. VOICEVOX のデフォルト値

項目ごとに上書きされます。例えば `vvsay -p zundamon -s あまあま` は、プリセットのパラメータのままスタイルだけ「あまあま」になります。

## コマンドラインオプション

```
使用方法: vvsay [OPTIONS] [TEXT]

引数:
  [TEXT]  読み上げるテキスト（または stdin からのパイプ）

オプション:
  -t, --text <FILE>                    読み上げるテキストファイル
  -o, --out <FILE>                     出力 WAV ファイルのパス（指定しない場合は mpv で再生）
  -n, --narrator <NAME>                キャラクター名（例: ずんだもん）
  -s, --style <NAME>                   スタイル名（省略時はそのキャラの最初のスタイル）
      --style-id <ID>                  スタイル ID（--list-narrator で確認）
  -p, --preset <NAME>                  音声プリセットを使用
      --speed <VALUE>                  話速（デフォルト 1.0）
      --pitch <VALUE>                  音高（デフォルト 0.0）
      --intonation <VALUE>             抑揚（デフォルト 1.0）
      --volume <VALUE>                 音量（デフォルト 1.0）
      --pre-phoneme-length <SECONDS>   音声の前の無音（秒、デフォルト 0.1）
      --post-phoneme-length <SECONDS>  音声の後の無音（秒、デフォルト 0.1）
      --list-narrator                  キャラクターとスタイルの一覧を表示
      --list-style <NARRATOR>          指定したキャラクターのスタイル一覧を表示
      --list-presets                   プリセットの一覧を表示
      --setup                          公式ダウンローダーで VOICEVOX CORE の実行ファイル類を導入（利用規約への同意を求められます）
      --accept-terms                   --setup と併用: 利用規約への同意確認に自動で回答（規約を読んだうえでのみ使用）
      --queue                          他の再生が終わるまで待ち、呼び出し順に再生
      --bg                             バックグラウンドで実行（即座に制御を返す）
  -v, --verbose                        詳細出力（VOICEVOX CORE のログと所要時間を表示）
  -h, --help                           ヘルプを表示
  -V, --version                        バージョンを表示
```

## ライセンス

このプロジェクトは MIT ライセンスです。詳細は [LICENSE](LICENSE) をご覧ください。
VOICEVOX CORE、VOICEVOX ONNX Runtime、音声モデルにはそれぞれの利用規約が適用されます。
